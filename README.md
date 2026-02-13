# NYC Subway Sign

![NYC Subway Sign Display](subway-sign.png)

A real-time MTA subway arrival display built with a Raspberry Pi 4 and LED matrix panels. Shows live train arrivals and service alerts using official GTFS-RT feeds.

## Features

- **Real-time arrivals** from MTA GTFS-RT feeds with 20-second refresh
- **Service alerts** with scrolling ticker display, triggered when trains arrive
- **Multi-platform support** for complex stations (e.g., Times Square with 8+ platforms)
- **Automatic station detection** via fuzzy name matching against 472-station database
- **Web control interface** for remote configuration without SSH
- **Hot-reload configuration** — changes apply within 5 seconds, no restart needed
- **Single binary** — one process replaces a multi-service Python stack

## Tech Stack

**Hardware:** Raspberry Pi 4 | 3x 64x32 LED Matrix Panels (192x32 total)

**Software:** Rust | Tokio | Axum | Prost (protobuf) | rpi-rgb-led-matrix

## Hardware Requirements

- Raspberry Pi 4 (2GB+ RAM recommended)
- 3x 64x32 RGB LED Matrix panels (HUB75 interface)
- 5V 20A power supply for LED panels
- Adafruit RGB Matrix Bonnet or equivalent HAT

## Quick Start

### 1. Clone and Configure

```bash
git clone https://github.com/nkocher/subway-sign.git
cd subway-sign

# Copy example config
cp config.example.json config.json

# Edit with your station
nano config.json
```

### 2. Build

```bash
# On Raspberry Pi (native compilation, requires root for GPIO)
cargo build --release --features hardware --no-default-features

# On Mac/Linux for development (uses mock display)
cargo build
```

### 3. Run

```bash
# On Raspberry Pi
sudo ./target/release/subway-sign

# On Mac/Linux (mock display, no hardware needed)
cargo run
```

The web interface starts automatically at `http://<pi-ip>:5001`.

## Configuration

Edit `config.json` to set your station:

```json
{
  "station": {
    "station_name": "Times Sq-42 St",
    "routes": ["1", "2", "3", "7", "N", "Q", "R", "W", "S"]
  },
  "display": {
    "brightness": 0.3,
    "max_trains": 7,
    "show_alerts": true
  }
}
```

The `station_name` field uses fuzzy matching — try names like:
- `"34 St-Herald Sq"` (Herald Square)
- `"Grand Central-42 St"` (Grand Central)
- `"Times Sq-42 St"` (Times Square)

## Architecture

```
Tokio async runtime
├── Fetch task (trains every 20s, alerts every 60s)
├── Config watcher (polls file mtime every 5s)
└── Web server (axum on port 5001)

Dedicated OS thread
└── Render loop (60fps, FFI to LED matrix hardware)

Shared state (lock-free)
├── ArcSwap<Config>
└── ArcSwap<DisplaySnapshot>
```

Key design principles:
- **Lock-free reads** — ArcSwap for config and display data, no locks in the render path
- **Immutable snapshots** — fetch tasks produce complete `DisplaySnapshot` values, eliminating race conditions
- **Bulk FFI** — single `set_image()` call per frame instead of per-pixel writes

## Project Structure

```
subway-sign/
├── Cargo.toml          # Dependencies and feature flags
├── config.json         # Your configuration (gitignored)
├── src/
│   ├── main.rs         # Entry point, task orchestration, render loop
│   ├── config.rs       # Configuration loading and validation
│   ├── models.rs       # Train, Alert, DisplaySnapshot types
│   ├── display/        # Rendering engine, fonts, framebuffer, LED matrix
│   ├── mta/            # GTFS-RT client, alert manager, station database
│   └── web/            # Axum web server and API handlers
├── assets/             # Fonts, icons, station database (compiled into binary)
├── proto/              # GTFS-RT protobuf schema
└── web/                # Static web UI (compiled into binary via rust-embed)
```

## Deployment

Example files are provided:
- `deploy.example.sh` — deployment script template (rsync + build on Pi)
- `systemd/subway-sign-rust.service.example` — systemd unit file

```bash
# Copy and configure deploy script
cp deploy.example.sh deploy.sh
# Edit PI_HOST and PI_PATH, then:
./deploy.sh
```

## Acknowledgments

- **[MTA](https://new.mta.info/)** — Real-time subway data via GTFS-RT feeds
- **[ColeWorks](https://www.coleworks.co/)** — MTA Countdown Clock font (CC0 license)
- **[hzeller/rpi-rgb-led-matrix](https://github.com/hzeller/rpi-rgb-led-matrix)** — LED matrix driver
- **[Claude](https://claude.ai)** — AI pair programming

## License

MIT License — see [LICENSE](LICENSE) for details.
