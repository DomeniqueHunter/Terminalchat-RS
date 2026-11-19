#!/usr/bin/env bash
# Stops the terminalchat tmux server. A running session keeps its old environment and its old
# binary, so stop it before starting a freshly built client.
set -euo pipefail

SOCKET="terminalchat-rs"

if ! command -v tmux >/dev/null 2>&1; then
    echo "tmux not found - nothing to stop"
    exit 0
fi

if env -u TMUX tmux -L "${SOCKET}" kill-server 2>/dev/null; then
    echo "tmux session '${SOCKET}' stopped"
else
    echo "no tmux session '${SOCKET}' running"
fi
