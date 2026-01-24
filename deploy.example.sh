#!/bin/bash
# Deploy NYC Subway Sign to Raspberry Pi
# Usage: ./deploy.sh
#
# Copy this file to deploy.sh and update the variables below:
#   PI_HOST: SSH user and IP/hostname of your Pi (e.g., admin@192.168.1.100)
#   PI_PATH: Installation path on the Pi

set -e

PI_HOST="YOUR_USERNAME@YOUR_PI_IP"
PI_PATH="/home/YOUR_USERNAME/subway-sign"
SERVICE_NAME="subway-sign.service"

echo "=== Deploying NYC Subway Sign ==="

# Sync files (excluding unnecessary items)
echo "Syncing files to Pi..."
rsync -avz --delete \
    --exclude '__pycache__' \
    --exclude '*.pyc' \
    --exclude '.git' \
    --exclude '.DS_Store' \
    --exclude 'venv' \
    --exclude 'tests' \
    --exclude '.claude' \
    --exclude 'archive' \
    --exclude 'tools' \
    --exclude 'TODO.md' \
    ./ ${PI_HOST}:${PI_PATH}/

# Install/update systemd services
echo "Updating systemd services..."
ssh ${PI_HOST} "sudo cp ${PI_PATH}/systemd/subway-sign.service /etc/systemd/system/ && sudo cp ${PI_PATH}/systemd/subway-web.service /etc/systemd/system/ && sudo systemctl daemon-reload"

# Restart main service
echo "Restarting subway-sign service..."
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
