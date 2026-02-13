#!/bin/bash
# Deploy NYC Subway Sign (Rust) to Raspberry Pi
# Usage: ./deploy.sh
#
# Copy this file to deploy.sh and update the variables below:
#   PI_HOST: SSH user and IP/hostname of your Pi (e.g., admin@192.168.1.100)
#   PI_PATH: Installation path on the Pi
#
# Build first:
#   cargo build --release --target aarch64-unknown-linux-gnu --features hardware --no-default-features

set -e

PI_HOST="YOUR_USERNAME@YOUR_PI_IP"
PI_PATH="/home/YOUR_USERNAME/subway-sign"
SERVICE_NAME="subway-sign-rust.service"
BINARY="target/aarch64-unknown-linux-gnu/release/subway-sign"

echo "=== Deploying NYC Subway Sign (Rust) ==="

# Check that binary exists
if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found at $BINARY"
    echo "Build first: cargo build --release --target aarch64-unknown-linux-gnu --features hardware --no-default-features"
    exit 1
fi

# Copy binary
echo "Deploying binary to Pi..."
scp "$BINARY" "${PI_HOST}:${PI_PATH}/subway-sign"

# Copy config if it doesn't exist on Pi
ssh ${PI_HOST} "test -f ${PI_PATH}/config.json || echo '{}' > ${PI_PATH}/config.json"

# Install/update systemd service (single service replaces two Python services)
echo "Updating systemd service..."
scp systemd/subway-sign-rust.service "${PI_HOST}:/tmp/subway-sign-rust.service"
ssh ${PI_HOST} "sudo mv /tmp/subway-sign-rust.service /etc/systemd/system/ && sudo systemctl daemon-reload"

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
