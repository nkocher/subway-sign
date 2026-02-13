# NYC Subway Sign - Project Guide

## Architecture

Single Rust binary running on a 4GB Raspberry Pi. tokio async runtime for I/O tasks, plus a dedicated OS thread for the render loop:

- **Main async runtime (tokio):** Fetch task (MTA GTFS-RT, 20s interval), alert task (60s interval), config watcher (5s poll), web server (axum on port 5001).
- **Render thread (std::thread):** Dedicated OS thread running the 60fps render loop. Not a tokio task because it runs perpetually with precise timing and makes blocking FFI calls to the LED matrix hardware.
- **Shared state:** `Arc<AppState>` with `ArcSwap<Config>` and `ArcSwap<DisplaySnapshot>` for lock-free reads. Config hot-reloads within 5 seconds when `config.json` changes.

## Concurrency

- **Lock-free reads:** `ArcSwap` for config and display snapshot. Render thread and web handlers read without blocking.
- **Dedicated render thread:** `std::thread::spawn` (not `tokio::spawn_blocking`). Runs perpetually at 60fps, calls FFI `set_image()` for bulk pixel updates.
- **Hardware FFI:** Direct call to hzeller's `set_image()` C API. Reduces per-frame overhead from 6,144 FFI calls (per-pixel) to 1 bulk copy.

## Display

- **DisplayTarget trait:** `hardware` feature compiles `LedMatrixDisplay` (Pi GPIO), `mock` feature (default) compiles `MockDisplay` (no-op, for macOS dev).
- **Render loop:** 60fps, 1px/frame scrolling. `set_image()` bulk FFI call keeps render thread at ~5% CPU. hzeller's GPIO thread uses ~73% CPU (expected, hard real-time PWM).
- **Panel layout:** 3 chained 64x32 panels = 192x32 total. PWM bits: 11, LSB nanoseconds: 130, GPIO slowdown: 3. Hardware pulsing enabled (`set_hardware_pulsing(true)`) — required for stable display; the crate defaults to software pulsing which causes visible jitter.
- **Stats line** (every 5min): FPS, missed frames (work exceeded 16.67ms budget), avg/max frame time, train/alert counts.

## Web

- **axum server** (replaces Flask/gunicorn). Runs on 0.0.0.0:5001.
- **rust-embed** for static files (HTML, CSS, JS, icons). All web assets compiled into the binary.
- **API endpoints:** `/api/config` (GET/POST), `/api/status`, `/api/stations/complete`, `/api/debug/snapshot`, `/api/restart`.

## Font System

- **Pre-decoded at load time.** All character bitmaps, widths, and left-padding are computed once during `MtaFont::load()` and stored in HashMaps. Zero per-frame heap allocations for text rendering.
- **`get_char_bitmap` returns `Option<&CharBitmap>`** (borrowed reference), not owned. Callers should not add `&` when passing to `blit_char`.
- **Space character special case.** Space has all-zero rows in font JSON — bitmap width computes to 1. Width is hardcoded to 4 during pre-computation.
- **Italic fallback.** If italic variant doesn't exist for a char, accessors fall back to regular. This applies to HashMap lookups too (use `.or_else()`).
- **LSB-first for chars, MSB-first for icons.** Two different bit orderings in the same font file.

## Build

**macOS dev (mock display):**
```bash
cargo build
cargo test
cargo clippy
```

**Clippy note:** ~24 warnings from generated protobuf code (`transit_realtime.rs`) are expected. Only `src/` warnings matter.

**Pi hardware (native compilation):**
```bash
cargo build --release --features hardware --no-default-features
```

Hardware builds emit 2 warnings about unused `MockDisplay` — this is expected (mock code is excluded by feature flags but still compiled).

**Pi build times:** Full rebuild ~4-5 min (ARM CPU). Dep changes trigger full rebuild. Use `timeout: 600000` on deploy Bash commands.

Binary location: `target/release/subway-sign`

Cross-compilation not supported. Build directly on the Pi.

## Key Numbers

- **Memory:** ~18MB RSS (vs Python's 54MB + 50MB). Single binary replaces two systemd services.
- **CPU:** Render thread ~5%, hzeller GPIO thread ~73% (expected).
- **Intervals:** Train fetch 20s, alert fetch 60s, config poll 5s, stats log 300s.
- **Render:** 60fps, 1px/frame scroll. Alert display triggered on train arrival (minutes == 0).
- **Tests:** 73 pass, 2 ignored (PPM visual tests that write to /tmp).

## Stability Patterns

Most Python-era patterns still apply:

- **Always handle SIGTERM.** Systemd sends SIGTERM on stop/restart. Rust port listens for SIGTERM and SIGINT via `shutdown_signal()` and uses `CancellationToken` to gracefully shut down all tasks.
- **Always set systemd `MemoryMax` and `TimeoutStopSec`.** Without limits, OOM killer may terminate sshd before the display process, making the Pi unreachable.
- **Keep logging minimal.** Pi has limited storage. Stats print every 5 minutes. Fetch logs only on train count change.
- **Render thread uses std::thread, not tokio.** It runs perpetually with precise timing and makes blocking FFI calls. `spawn_blocking` is for short-lived operations.
- **set_image() bulk FFI.** Direct C API call copies entire framebuffer in one operation. Per-pixel FFI would be 6,144 calls/frame.

New patterns:

- **Lock-free reads via ArcSwap.** Config and snapshot updates never block readers. Render thread and web handlers always see consistent snapshots.
- **hzeller GPIO thread CPU usage.** The `rpi-led-matrix` library spawns its own thread for PWM. ~73% CPU is expected behavior for hard real-time LED refresh.

## Deployment

**Pi:** `admin@192.168.0.40` at `/home/admin/subway-sign-rust`
**OS:** Debian Trixie (Rust toolchain via rustup)

### Deploy workflow

```bash
# 1. Deploy code and build on Pi
./deploy.sh

# 2. Restart systemd service (single service, not two)
ssh admin@192.168.0.40 "sudo systemctl restart subway-sign.service"
```

`deploy.sh` is gitignored — create it from `deploy.example.sh` with `PI_HOST="admin@192.168.0.40"` and `PI_PATH="/home/admin/subway-sign-rust"`.

`deploy.sh` rsyncs from CWD — must run from the worktree root, not the parent project.

The deploy script should:
1. rsync code to Pi
2. ssh to Pi and run `cargo build --release --features hardware --no-default-features`
3. Copy binary to expected location or update systemd service to point to `target/release/subway-sign`

### systemd service

Replace the two Python services (`subway-sign.service`, `subway-web.service`) with a single Rust service:

```ini
[Unit]
Description=NYC Subway Sign (Rust)
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/home/admin/subway-sign-rust
ExecStart=/home/admin/subway-sign-rust/target/release/subway-sign
Restart=always
RestartSec=5

# Stability limits
MemoryMax=256M
TimeoutStopSec=15s
OOMScoreAdjust=-500

# Real-time priority for render thread
Nice=-10
CPUSchedulingPolicy=fifo

[Install]
WantedBy=multi-user.target
```

### Verification commands

```bash
# Check systemd memory limit
systemctl show subway-sign.service -p MemoryMax  # expect 268435456 (256M)

# Test graceful shutdown
sudo systemctl restart subway-sign && journalctl -u subway-sign -n 20

# Memory snapshot
ps -o pid,rss,vsz,comm -p $(pgrep subway-sign)

# CPU usage (expect render thread ~5%, GPIO thread ~73%)
top -p $(pgrep subway-sign)

# Check web server
curl http://192.168.0.40:5001/api/status
```

### Verified baselines (pending)

Rust port deployed but **24-hour stability test still pending**. Expected baselines:

- **RSS:** ~18MB (within 256M MemoryMax)
- **SIGTERM shutdown:** Clean shutdown via `CancellationToken`, all tasks joined
- **OOMScoreAdjust:** -500 (OOM killer targets other processes first, but not sshd)
- **Render FPS:** 60fps sustained
- **GPIO thread CPU:** ~73% (expected hard real-time PWM overhead)
