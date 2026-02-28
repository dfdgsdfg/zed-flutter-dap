#!/usr/bin/env bash
#
# flutter-reload.sh — Send commands to the dap-proxy.
#
# Usage:
#   flutter-reload.sh reload          # Hot reload
#   flutter-reload.sh restart         # Hot restart
#   flutter-reload.sh devtools        # Open Flutter DevTools in browser
#   flutter-reload.sh status          # Show VM service URI
#   flutter-reload.sh <command>       # Send any DAP command

set -euo pipefail

COMMAND="${1:-reload}"

# Find socket path
SOCKET=""
if [ -f /tmp/zed-dap-latest ]; then
    SOCKET="$(cat /tmp/zed-dap-latest)"
    if [ ! -S "$SOCKET" ]; then
        SOCKET=""
    fi
fi

if [ -z "$SOCKET" ]; then
    SOCKET="$(ls -t /tmp/zed-dap-*.sock 2>/dev/null | head -1)" || true
fi

if [ -z "$SOCKET" ] || [ ! -S "$SOCKET" ]; then
    echo "Error: No dap-proxy socket found. Start a Flutter debug session first." >&2
    exit 1
fi

# Map shorthand names to DAP commands
case "$COMMAND" in
    reload)   DAP_CMD="hotReload" ;;
    restart)  DAP_CMD="hotRestart" ;;
    devtools) DAP_CMD="devtools" ;;
    status)   DAP_CMD="status" ;;
    *)        DAP_CMD="$COMMAND" ;;
esac

echo "Sending $DAP_CMD via $SOCKET..."
RESPONSE=$(echo "{\"command\": \"$DAP_CMD\"}" | nc -U "$SOCKET")

# For devtools command, open the URL in the default browser
if [ "$DAP_CMD" = "devtools" ]; then
    URL=$(echo "$RESPONSE" | python3 -c "import sys,json; print(json.load(sys.stdin).get('devtoolsUrl',''))" 2>/dev/null || true)
    if [ -n "$URL" ]; then
        echo "Opening DevTools: $URL"
        open "$URL" 2>/dev/null || xdg-open "$URL" 2>/dev/null || echo "$URL"
    else
        echo "$RESPONSE"
    fi
else
    echo "$RESPONSE"
fi
