#!/bin/bash

pids="$(pgrep -d, -f '^[^ ]*terminalchat-(x86_64|aarch64)( |$)' || true)"
if [[ -z "${pids}" ]]; then
    echo "terminalchat is not running" >&2
    exit 1
fi

date '+%Y-%m-%d %H:%M:%S'; ps -o pid,rss,%cpu,%mem,etime,cmd -p "${pids}"
