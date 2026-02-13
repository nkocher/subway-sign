#!/bin/bash
# Deploy NYC Subway Sign (Rust) to Raspberry Pi
# Usage: ./deploy.sh
#
# Copy this file to deploy.sh and update the variables below:
#   PI_HOST: SSH user and IP/hostname of your Pi (e.g., admin@192.168.1.100)
#   PI_PATH: Installation path on the Pi
#
# The binary is built natively on the Pi:
#   cargo build --release --features hardware --no-default-features

set -e

PI_HOST="YOUR_USERNAME@YOUR_PI_IP"
PI_PATH="/home/YOUR_USERNAME/subway-sign-rust"
SERVICE_NAME="subway-sign-rust.service"

echo "=== Deploying NYC Subway Sign (Rust) ==="

# Sync source code to Pi
echo "Syncing source to Pi..."
rsync -az --delete \
    --exclude target/ \
    --exclude .git/ \
    --exclude .worktrees/ \
    --exclude deploy.sh \
    ./ "${PI_HOST}:${PI_PATH}/"

# Build on Pi (native compilation)
echo "Building on Pi..."
ssh ${PI_HOST} "cd ${PI_PATH} && cargo build --release --features hardware --no-default-features"

# Copy config if it doesn't exist on Pi
ssh ${PI_HOST} "test -f ${PI_PATH}/config.json || cp ${PI_PATH}/config.example.json ${PI_PATH}/config.json"

# Install/update systemd service
echo "Updating systemd service..."
ssh ${PI_HOST} "sudo cp ${PI_PATH}/systemd/subway-sign-rust.service.example /etc/systemd/system/subway-sign-rust.service && sudo systemctl daemon-reload"

# Restart service
echo "Restarting ${SERVICE_NAME}..."
ssh ${PI_HOST} "sudo systemctl restart ${SERVICE_NAME}"

# Check status
echo ""
echo "=== Service Status ==="
ssh ${PI_HOST} "sudo systemctl status ${SERVICE_NAME} --no-pager" || true

echo ""
echo "=== Deployment complete ==="
echo ""
echo "Useful commands:"
echo "  Monitor logs:     ssh ${PI_HOST} 'sudo journalctl -u ${SERVICE_NAME} -f'"
echo "  Check status:     ssh ${PI_HOST} 'sudo systemctl status ${SERVICE_NAME}'"
echo "  Restart service:  ssh ${PI_HOST} 'sudo systemctl restart ${SERVICE_NAME}'"
echo "  Memory check:     ssh ${PI_HOST} 'ps -o pid,rss,comm -p \$(pgrep subway-sign)'"
