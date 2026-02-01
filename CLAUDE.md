# NYC Subway Sign - Project Guide

## Architecture

Two systemd services running on a 4GB Raspberry Pi:

- **subway-sign.service** — Display process (`src/main.py`). Fetches MTA GTFS-RT data via `src/mta/client.py`, renders to LED matrix at 30fps. Runs as root with real-time CPU priority (`Nice=-10`, `CPUSchedulingPolicy=fifo`).
- **subway-web.service** — Web UI (`web/app.py`). Flask app served by gunicorn (2 workers, auto-recycled at 1000 requests). Runs as `admin` user. Config changes hot-reload in the display process within 5 seconds.

## Stability Patterns

These patterns exist to prevent the Pi from becoming unreachable after extended uptime:

- **Always handle SIGTERM** in long-running services. Systemd sends SIGTERM on stop/restart, not SIGINT. Without a handler, ThreadPoolExecutor threads, HTTP sessions, and sockets leak on every restart cycle.
- **Never use Flask dev server in production.** Use gunicorn with `--max-requests` for automatic worker recycling.
- **Always set systemd `MemoryMax` and `TimeoutStopSec`** for Pi services. Without limits, the OOM killer will terminate sshd (low priority) while sparing the display service (high priority), making the Pi unreachable.
- **Cache subprocess calls, never spawn per-request.** The web UI polls `/api/status` every 30s; without caching, this spawns ~5,760 subprocesses/day per open tab.
- **Keep logging minimal.** Pi has limited storage. Stats print every 5 minutes (not 30 seconds). Error messages are rate-limited to one per source per 5 minutes.
- **Keep `max_workers=8`** for the ThreadPoolExecutor — this matches the number of MTA GTFS-RT feeds fetched in parallel.

## Key Numbers

- Display process baseline memory: ~54MB RSS (stable)
- `MemoryMax`: 512M (display), 256M (web)
- Connection pools: 4 connections, 8 max size (matches feed count)
- Stats interval: 300s. Fetch log: only on train count change.

## Development

```bash
# Syntax check all modified Python files
python3 -m py_compile src/main.py
python3 -m py_compile web/app.py
python3 -m py_compile src/mta/client.py

# Run web UI locally (dev server)
cd web && python3 app.py

# Run web UI locally (production-like)
cd web && gunicorn --bind 127.0.0.1:5001 --workers 1 --timeout 5 app:app
```

## Deployment

**Pi:** `admin@192.168.0.40` at `/home/admin/subway-sign`

### Prerequisites

Gunicorn must be installed on the Pi via apt (not pip):

```bash
ssh admin@192.168.0.40 "sudo apt install -y gunicorn"
```

### Deploy workflow

```bash
# 1. Deploy code and restart subway-sign
./deploy.sh

# 2. Restart subway-web (deploy.sh only restarts subway-sign)
ssh admin@192.168.0.40 "sudo systemctl restart subway-web.service"
```

`deploy.sh` is gitignored — create it from `deploy.example.sh` with `PI_HOST="admin@192.168.0.40"` and `PI_PATH="/home/admin/subway-sign"`.

### Verified baselines (2026-02-01)

Stability fixes deployed and verified:

- **subway-sign RSS:** ~54MB (within 512M MemoryMax)
- **gunicorn master RSS:** ~23MB, workers: ~50MB each (within 256M MemoryMax)
- **OOMScoreAdjust:** -500 (display), +500 (web) — OOM killer targets web before display/sshd
- **SIGTERM shutdown:** Clean "Deactivated successfully" — no forced kills, no leaked threads
- **Gunicorn:** 1 master + 2 workers, auto-recycled at 1000 requests

### Verification commands

```bash
# Check systemd memory limits
systemctl show subway-sign.service -p MemoryMax  # expect 536870912
systemctl show subway-web.service -p MemoryMax   # expect 268435456

# Test graceful shutdown
sudo systemctl restart subway-sign && journalctl -u subway-sign -n 20

# Check gunicorn processes (expect master + 2 workers)
pgrep -a gunicorn

# Memory snapshot
ps -o pid,rss,vsz,comm -p $(pgrep -f 'run.py') $(pgrep -f 'gunicorn')

# Long-running stability test
python3 tests/stability_monitor.py  # Run for 24+ hours
```
