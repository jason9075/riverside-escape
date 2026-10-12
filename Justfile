default: list

list:
    @just --list

server:
    cargo run -p riverside-server

client:
    NO_COLOR=true trunk serve

dev:
    #!/usr/bin/env bash
    set -euo pipefail
    RIVERSIDE_DEV_SOLO=1 cargo run -p riverside-server &
    server_pid=$!
    cleanup() {
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    }
    trap cleanup EXIT
    NO_COLOR=true trunk serve
