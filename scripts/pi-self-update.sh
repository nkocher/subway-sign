#!/bin/bash
# Weekly self-update for the subway sign, run by subway-sign-update.timer.
#
# Pulls main from GitHub, rebuilds, restarts the service, and health-checks.
# On a failed health check, restores the previous binary. A failed build
# leaves the running binary untouched (cargo only replaces it on success).
#
# Runs as user `admin` (needs passwordless sudo for systemctl restart).

set -euo pipefail

REPO_DIR="/home/admin/subway-sign-rust"
BINARY="$REPO_DIR/target/release/subway-sign"
BACKUP="$REPO_DIR/target/release/subway-sign.prev"
SERVICE="subway-sign-rust.service"
HEALTH_URL="http://localhost:5001/api/healthz"

# Entire logic lives in main() so bash parses the whole file before running
# anything — this script replaces itself via git during execution.
main() {
    cd "$REPO_DIR"
    export PATH="$HOME/.cargo/bin:$PATH"

    git fetch origin main
    local head remote
    head=$(git rev-parse HEAD)
    remote=$(git rev-parse origin/main)

    # Keep the toolchain current even when there's no code change.
    rustup update || echo "WARN: rustup update failed, building with current toolchain"

    if [ "$head" = "$remote" ]; then
        echo "Already up to date ($head)"
        exit 0
    fi

    echo "Updating $head -> $remote"
    git reset --hard origin/main

    if [ -f "$BINARY" ]; then
        cp "$BINARY" "$BACKUP"
    fi

    cargo build --release --features hardware --no-default-features

    sudo systemctl restart "$SERVICE"
    sleep 20

    if curl -fsS --max-time 10 "$HEALTH_URL" > /dev/null; then
        echo "Update OK: now running $remote"
        exit 0
    fi

    echo "ERROR: health check failed after update, rolling back to previous binary"
    if [ -f "$BACKUP" ]; then
        cp "$BACKUP" "$BINARY"
        sudo systemctl restart "$SERVICE"
    fi
    exit 1
}

main "$@"
