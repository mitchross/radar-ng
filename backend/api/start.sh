#!/bin/sh
set -e

# Start Python in background, tee output to a log file so crash traces survive
# Caddy foregrounding the process. Without this, Python's stderr is swallowed.
# Loopback only: Caddy is the sole entrypoint (error handling, no-store headers).
python -m uvicorn backend.api.api.server:app --host 127.0.0.1 --port 8000 2>&1 | tee /tmp/uvicorn.log &

# Wait for uvicorn to bind before Caddy starts proxying
for i in $(seq 1 30); do
  if python -c "import socket; socket.create_connection(('127.0.0.1', 8000), timeout=1)" 2>/dev/null; then break; fi
  sleep 1
done

caddy run --config /etc/caddy/Caddyfile --adapter caddyfile
