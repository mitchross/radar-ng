#!/usr/bin/env bash
# API load comparison (bench 5): Python (FastAPI + StaticFiles stand-in for Caddy) vs Rust radar-api.
#
# Both servers get the same fixture (make_api_fixture.py), the same request list (mix.py, local
# endpoints only), the same core budget: server pinned to SERVER_CPUS, oha and the /proc sampler
# pinned to OHA_CPUS. Each target runs fixed-rate steps (RATES) and one max-throughput run (-c CONC).
#
# Usage:  bench/load/run_load.sh                  (fixture must exist: make_api_fixture.py)
# Env:
#   FIXTURE          fixture root (default bench/work/api_fixture)
#   API_PYTHON       venv with backend/api/api/requirements.txt (default bench/.venv-api/bin/python)
#   RADAR_NG         original Python tree (default .)
#   RUST_ROOT        Rust workspace (default bench/../rust); binary is $CARGO_TARGET_DIR/release/bench_api_server
#   RADAR_API_BIN    override the Rust server binary
#   OHA              oha binary (default $(which oha))
#   PY_PORT / RS_PORT  default 8081 / 8082
#   LOAD_SECONDS     per run (default 30)   RATES  fixed-rate steps (default "200 500 1000")
#   CONC             concurrency for rate runs and the max run (default 64)
#   SERVER_CPUS      cores for the server (default 0)   OHA_CPUS  cores for oha + sampler (default 1)
#   RATE_LIMIT_OFF  1 (default) lifts the per-IP limit (API_RATE_LIMIT_RPS=100000) so the test
#                   measures the handlers, not the limiter. Set 0 for the production limit (20 rps).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BENCH="$(cd "$HERE/.." && pwd)"
RUST_ROOT="${RUST_ROOT:-$BENCH/../rust}"
CARGO_TARGET="${CARGO_TARGET_DIR:-$RUST_ROOT/target}"
RADAR_NG="${RADAR_NG:-$(cd "$BENCH/.." && pwd)}"
FIXTURE="${FIXTURE:-$BENCH/work/api_fixture}"
TILE_DIR="$FIXTURE/tiles"
STATE_DIR="$FIXTURE/state"
GRID_DIR="$FIXTURE/grids"
PY_PORT="${PY_PORT:-8081}"
RS_PORT="${RS_PORT:-8082}"
RADAR_API_BIN="${RADAR_API_BIN:-$CARGO_TARGET/release/bench_api_server}"
OHA="${OHA:-$(which oha)}"
LOAD_SECONDS="${LOAD_SECONDS:-30}"
RATES="${RATES:-200 500 1000}"
CONC="${CONC:-64}"
SERVER_CPUS="${SERVER_CPUS:-0}"
OHA_CPUS="${OHA_CPUS:-1}"
PY="${API_PYTHON:-$BENCH/.venv-api/bin/python}"
RATE_LIMIT_OFF="${RATE_LIMIT_OFF:-1}"
LIMIT=20
[ "$RATE_LIMIT_OFF" = "1" ] && LIMIT=100000
TS="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="$BENCH/results/load/$TS"
mkdir -p "$OUT"

for f in "$TILE_DIR" "$STATE_DIR/manifest.json" "$GRID_DIR"; do
  [ -e "$f" ] || { echo "missing fixture path $f (run load/make_api_fixture.py)" >&2; exit 1; }
done
[ -x "$OHA" ] || { echo "oha not found at $OHA" >&2; exit 1; }
[ -x "$RADAR_API_BIN" ] || { echo "rust server not built: $RADAR_API_BIN (cargo build --release -p radar-bench --bin bench_api_server)" >&2; exit 1; }
command -v taskset >/dev/null || { echo "taskset required for core pinning" >&2; exit 1; }

# Request list from the served manifest, same for both servers (Rust port substituted below).
"$PY" "$HERE/mix.py" --base "http://127.0.0.1:$PY_PORT" --manifest "$STATE_DIR/manifest.json" \
  --tiles-root "$TILE_DIR" --out "$OUT/urls_py.txt"
sed "s|http://127.0.0.1:$PY_PORT|http://127.0.0.1:$RS_PORT|g" "$OUT/urls_py.txt" > "$OUT/urls_rs.txt"

wait_live() {
  local port=$1
  for _ in $(seq 1 120); do
    if curl -fsS "http://127.0.0.1:$port/api/livez" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  return 1
}

run_target() {
  local name=$1 pid=$2 urls=$3
  for rate in $RATES; do
    taskset -c "$OHA_CPUS" "$OHA" -z "${LOAD_SECONDS}s" -q "$rate" -c "$CONC" --no-tui --output-format json \
      -o "$OUT/${name}_rate${rate}.oha.json" --urls-from-file "$urls" 2> "$OUT/${name}_rate${rate}.err" &
    local oha_pid=$!
    taskset -c "$OHA_CPUS" "$PY" "$HERE/sample_proc.py" --pid "$pid" --seconds "$((LOAD_SECONDS + 2))" \
      --out "$OUT/${name}_rate${rate}.proc.json"
    wait "$oha_pid" || true
  done
  taskset -c "$OHA_CPUS" "$OHA" -z "${LOAD_SECONDS}s" -c "$CONC" --no-tui --output-format json \
    -o "$OUT/${name}_max.oha.json" --urls-from-file "$urls" 2> "$OUT/${name}_max.err" &
  local oha_pid=$!
  taskset -c "$OHA_CPUS" "$PY" "$HERE/sample_proc.py" --pid "$pid" --seconds "$((LOAD_SECONDS + 2))" \
    --out "$OUT/${name}_max.proc.json"
  wait "$oha_pid" || true
}

SERVER_PID=""
cleanup() { [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

start_python() {
  TILE_DIR="$TILE_DIR" STATE_DIR="$STATE_DIR" GRID_DIR="$GRID_DIR" RADAR_NG="$RADAR_NG" \
  API_RATE_LIMIT_RPS="$LIMIT" API_RATE_LIMIT_BURST="$LIMIT" \
    taskset -c "$SERVER_CPUS" "$PY" -m uvicorn --app-dir "$HERE" py_server:app \
      --host 127.0.0.1 --port "$PY_PORT" --workers 1 --no-access-log \
      > "$OUT/python_server.log" 2>&1 &
  SERVER_PID=$!
}

start_rust() {
  LISTEN_ADDR="127.0.0.1:$RS_PORT" TILE_DIR="$TILE_DIR" STATE_DIR="$STATE_DIR" GRID_DIR="$GRID_DIR" \
  OPEN_METEO_BASE="http://127.0.0.1:9/v1/forecast" \
  API_RATE_LIMIT_RPS="$LIMIT" API_RATE_LIMIT_BURST="$LIMIT" \
    taskset -c "$SERVER_CPUS" "$RADAR_API_BIN" > "$OUT/rust_server.log" 2>&1 &
  SERVER_PID=$!
}

# Python
start_python
PY_PID=$SERVER_PID
if wait_live "$PY_PORT"; then
  run_target python "$PY_PID" "$OUT/urls_py.txt"
else
  echo "python server did not become live; see $OUT/python_server.log" >&2
fi
cleanup; SERVER_PID=""

# Rust
start_rust
RS_PID=$SERVER_PID
if wait_live "$RS_PORT"; then
  run_target rust "$RS_PID" "$OUT/urls_rs.txt"
else
  echo "rust server did not become live; see $OUT/rust_server.log" >&2
fi
cleanup; SERVER_PID=""

echo "results in $OUT"
