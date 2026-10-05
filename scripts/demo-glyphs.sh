#!/bin/bash
# Step the sign through every alert glyph and the clock face, for sign-off on
# the physical display. Uses POST /api/debug/preview; nothing is persisted.
#
# Usage: scripts/demo-glyphs.sh [seconds-per-message]   (default 25)

set -euo pipefail

SIGN="${SIGN:-http://ledpi.local:5001}"
SECONDS_EACH="${1:-25}"

MESSAGES=(
    "[airplane icon] JFK AirTrain service is suspended and replaced by free shuttle buses"
    "Take [shuttle bus icon] free shuttle buses and/or rerouted [A] via [H]"
    "No [SIR] trains between St George and Tottenville"
    "[FX] [6X] [7X] run express; [A] [6] [7] run local"
    "Enter 5 Av/53 St [E][F] Station (Madison Av entrance) & take the elevator [accessibility icon]"
)

preview() {
    curl -fsS -X POST "$SIGN/api/debug/preview" \
        -H 'Content-Type: application/json' -d "$1" >/dev/null
}

trap 'curl -fsS -X DELETE "$SIGN/api/debug/preview" >/dev/null || true' EXIT

for msg in "${MESSAGES[@]}"; do
    echo "alert: $msg"
    preview "$(jq -n --arg t "$msg" --argjson s "$SECONDS_EACH" '{alert: $t, seconds: $s}')"
    sleep "$SECONDS_EACH"
done

echo "clock face"
preview "{\"clock\": true, \"seconds\": $SECONDS_EACH}"
sleep "$SECONDS_EACH"
