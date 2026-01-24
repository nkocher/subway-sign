# Deployment Guide

## Quick Start (Raspberry Pi)

### Prerequisites

- Raspberry Pi 4 with Raspberry Pi OS
- rpi-rgb-led-matrix library installed
- LED matrix panels (3x 64x32 chained = 192x32)
- Network connectivity

### Deploy from Local Machine

1. Copy `deploy.example.sh` to `deploy.sh`
2. Edit `deploy.sh` and set your Pi's IP/hostname and username
3. Run:

```bash
./deploy.sh
```

This will:
1. Sync files to the Pi (excluding __pycache__, .git, etc.)
2. Restart the subway-sign service
3. Show service status

### Manual Installation

```bash
# 1. Copy project to Pi (replace YOUR_PI with your Pi's IP or hostname)
scp -r ./* YOUR_USERNAME@YOUR_PI:~/subway-sign/

# 2. SSH to Pi
ssh YOUR_USERNAME@YOUR_PI

# 3. Install dependencies
cd ~/subway-sign
pip install -r requirements.txt --break-system-packages

# 4. Configure your station
cp config.example.json config.json
nano config.json

# 5. Test run
sudo python3 run.py
# Should see display working
# Ctrl+C to stop
```

## Configuration

Edit `config.json` with your station information:

```json
{
  "station": {
    "stations": [
      {
        "uptown": "127N",
        "downtown": "127S"
      }
    ],
    "routes": ["1", "2", "3"]
  },
  "display": {
    "brightness": 0.75,
    "max_trains": 7,
    "show_alerts": true
  }
}
```

### Finding Your Stop IDs

MTA stop IDs follow this pattern:
- Last character is direction: `N` (north/uptown) or `S` (south/downtown)
- Without direction suffix, it's the complex/station ID

Examples:
- Times Sq-42 St (1/2/3): `127N` / `127S`
- Grand Central (4/5/6): `631N` / `631S`
- Union Square (L): `L03N` / `L03S`

Find IDs at: http://web.mta.info/developers/data/nyct/subway/Stations.csv

### Multiple Platforms

If your station has multiple platforms (like Times Square):

```json
{
  "station": {
    "stations": [
      {
        "uptown": "127N",
        "downtown": "127S"
      },
      {
        "uptown": "725N",
        "downtown": "725S"
      }
    ],
    "routes": ["1", "2", "3", "7", "N", "Q", "R", "W"]
  }
}
```

## Systemd Service

### Install Service

```bash
sudo cp ~/subway-sign/systemd/subway-sign.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable subway-sign.service
sudo systemctl start subway-sign.service
```

### Service Commands

```bash
# Start/stop/restart
sudo systemctl start subway-sign.service
sudo systemctl stop subway-sign.service
sudo systemctl restart subway-sign.service

# Check status
sudo systemctl status subway-sign.service

# View logs
sudo journalctl -u subway-sign.service -f        # Follow live
sudo journalctl -u subway-sign.service -n 100    # Last 100 lines
sudo journalctl -u subway-sign.service | grep ERROR  # Search for errors
```

## Testing Without Hardware

You can develop and test on macOS/Linux without the actual LED matrix:

```bash
cd ~/projects/subway-sign/production

# Install just the Python dependencies (matrix will fail, that's OK)
pip install Pillow requests nyct-gtfs

# Run with mock display
python3 run.py
```

The mock display mode will:
- Print initialization messages
- Fetch real MTA data
- Render frames (but not display them)
- Log statistics every 30s

## Troubleshooting

### "Permission denied" errors

Run with sudo:
```bash
sudo python3 run.py
```

The matrix library requires root for GPIO access.

### "Module not found" errors

```bash
pip install -r requirements.txt --break-system-packages

# Or install individually
pip install Pillow requests nyct-gtfs
```

### "Config file not found"

Make sure `config.json` exists in project root:
```bash
cd ~/subway-sign
ls -la config.json
```

### No trains showing

Check logs for API errors:
```bash
journalctl -u subway-sign.service -n 100
```

Common causes:
- Network connectivity issue
- Invalid stop IDs
- MTA API down (rare)

### Display flickering/strobing

This shouldn't happen with the simple timing approach. If it does:
1. Check brightness setting (try lower)
2. Check power supply (LED matrices draw lots of current)
3. Verify GPIO slowdown setting in code

## Performance Monitoring

### Check Memory Usage

```bash
ps aux | grep python3
# Expected: <50MB RSS
```

### Check CPU Usage

```bash
top -p $(pgrep -f subway-sign)
# Expected: <5% at idle, <15% during render
```

### Check Frame Rate

Logs print stats every 30 seconds:
```
[STATS] FPS: 29.8, Trains: 7, Alerts: 2
```

Target: 29-30 FPS consistently

### Check Network

```bash
curl https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs-l
# Should return binary data (GTFS-RT protobuf)
```

## Updating

### Deploy New Version

From local machine:
```bash
cd ~/projects/subway-sign/production
./deploy.sh
```

Or manually:
```bash
rsync -avz --delete \
    --exclude '__pycache__' \
    --exclude '*.pyc' \
    --exclude '.git' \
    --exclude '.DS_Store' \
    ./ YOUR_USERNAME@YOUR_PI:~/subway-sign/

ssh YOUR_USERNAME@YOUR_PI "sudo systemctl restart subway-sign.service"
```

## Hardware Setup

### LED Matrix Wiring

```
Pi GPIO → Matrix Hub75
----------------------------------------
Pin 2 (5V)     → 5V Power (external)
Pin 6 (GND)    → GND
Pin 8 (GPIO14) → R1
Pin 10 (GPIO15)→ G1
...
```

Full pinout: https://github.com/hzeller/rpi-rgb-led-matrix

### Power Requirements

3x 64x32 matrices at full brightness can draw 15A+ at 5V.

**Important:**
- Use adequate power supply (5V 20A recommended)
- Power matrices separately (don't power from Pi)
- Share ground between Pi and matrix power

### Matrix Chain Order

```
[Pi] → [Panel 1] → [Panel 2] → [Panel 3]
```

Set in code:
```python
options.chain_length = 3  # Number of panels
```

## Backup and Recovery

### Backup Configuration

```bash
cp ~/subway-sign/config.json ~/config-backup.json
```

### Backup Entire Project

```bash
# On Pi
tar -czf subway-sign-backup.tar.gz ~/subway-sign/

# Copy to local
scp YOUR_USERNAME@YOUR_PI:~/subway-sign-backup.tar.gz ~/backups/
```

## Security Considerations

### API Keys

The current implementation doesn't require an API key (uses public feeds).

If MTA adds authentication in future, add to config:
```json
{
  "api_key": "your-key-here"
}
```

### Network Access

The sign makes HTTPS requests to:
- `api-endpoint.mta.info` (MTA GTFS-RT API)

Ensure firewall allows outbound HTTPS.

## Pi Structure

After deployment, the Pi should have:

```
/home/admin/
├── subway-sign/          # Production code
│   ├── run.py
│   ├── config.json
│   ├── src/
│   ├── assets/
│   ├── web/
│   └── ...
└── (no archive - archived versions stay on local machine only)
```

Archived versions (V1, V2) are NOT deployed to the Pi to prevent confusion.
