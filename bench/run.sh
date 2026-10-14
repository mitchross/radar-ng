#!/usr/bin/env bash
# radar-ng before/after benchmark: runs every benchmark, cross-checks the outputs, and writes
#   bench/results/<UTC timestamp>.json   machine, raw per-iteration records, summaries, verification
#   bench/results/<UTC timestamp>.md     before/after tables with speedups and RSS ratios
#   bench/results/latest.md              copy of the newest markdown report
#
# Environment (all optional):
#   BENCH_PYTHON   Python with the packages in bench/py/requirements.txt (default bench/.venv/bin/python)
#   RUST_ROOT      Cargo workspace (default ../rust next to this script)
#   CARGO_TARGET_DIR  Cargo target directory (default: cargo's default for RUST_ROOT)
#   RADAR_NG       original Python tree, read-only (default .)
#   BENCH_WORK     scratch for inputs and outputs (default bench/work)
#   BENCH_ENV      optional shell file to source before cargo (for sandboxes that need proxy settings)
#   BENCH_QUICK=1  smoke run: one timed iteration, no warmup, no production-size nowcast
#   BENCH_SKIP=tile,grib,storms,nowcast   skip benchmark groups
set -euo pipefail

BENCH="$(cd "$(dirname "$0")" && pwd)"
RUST="${RUST_ROOT:-$BENCH/../rust}"
export BENCH_WORK="${BENCH_WORK:-$BENCH/work}"
export RADAR_NG="${RADAR_NG:-$(cd "$BENCH/.." && pwd)}"
export PYTHONDONTWRITEBYTECODE=1
PY="${BENCH_PYTHON:-$BENCH/.venv/bin/python}"
if [ ! -x "$PY" ]; then
  echo "note: $PY not found, using python3 (see bench/README.md for the venv)" >&2
  PY=python3
fi
if [ -n "${BENCH_ENV:-}" ]; then set +u; . "$BENCH_ENV"; set -u; fi
SKIP=",${BENCH_SKIP:-},"
want() { [[ "$SKIP" != *",$1,"* ]]; }

RAW="$BENCH_WORK/raw"
INPUTS="$BENCH_WORK/inputs"
OUT="$BENCH_WORK/out"
mkdir -p "$RAW" "$OUT"

# Iteration plan. Production-size nowcast is capped: one Python iteration takes minutes on 2 cores.
if [ "${BENCH_QUICK:-0}" = "1" ]; then
  WU=0; IT=1; NOW_SMALL_WU=0; NOW_SMALL_IT=1; NOW_PROD_WU=0; NOW_PROD_IT=0; GRIB_WU=0; GRIB_IT=2
else
  WU=1; IT=5; NOW_SMALL_WU=1; NOW_SMALL_IT=3; NOW_PROD_WU=0; NOW_PROD_IT=1; GRIB_WU=2; GRIB_IT=20
fi

# Machine load is sampled every 5 s while the benchmarks run, and each step is marked in
# steps.log, so the report can show what else was running during each measurement.
LOAD_LOG="$RAW/load.log"
STEPS_LOG="$RAW/steps.log"
: > "$LOAD_LOG"; : > "$STEPS_LOG"
( while true; do echo "$(date +%s) $(cut -d' ' -f1 /proc/loadavg)" >> "$LOAD_LOG"; sleep 5; done ) &
SAMPLER_PID=$!
trap 'kill $SAMPLER_PID 2>/dev/null || true' EXIT
step() { printf '\n== %s\n' "$*"; echo "$(date +%s) $*" >> "$STEPS_LOG"; }

step "Rust release build (radar-bench)"
(
  cd "$RUST"
  if [ -n "${BENCH_ENV:-}" ]; then set +u; . "$BENCH_ENV"; set -u; fi
  cargo build --release -p radar-bench
)
TARGET_DIR="${CARGO_TARGET_DIR:-$RUST/target}"
RBIN="$TARGET_DIR/release"
"$PY" "$BENCH/py/provenance.py" "$RUST" "$RBIN" "$RAW/provenance.json"

if [ ! -f "$INPUTS/manifest.json" ]; then
  step "Generate synthetic inputs (seeded, deterministic)"
  "$PY" "$BENCH/py/gen_inputs.py"
fi

if ! want tile; then echo "skip tile"; else
  step "Tile render: Python legacy (production default)"
  "$PY" "$BENCH/py/bench_tile.py" --renderer legacy --warmup "$WU" --iters "$IT" --json "$RAW/tile_py_legacy.json"
  step "Tile render: Python indexed (same algorithm as Rust)"
  "$PY" "$BENCH/py/bench_tile.py" --renderer indexed --warmup "$WU" --iters "$IT" --json "$RAW/tile_py_indexed.json"
  step "Tile render: Rust"
  "$RBIN/bench_tile" --input-dir "$INPUTS" --out-root "$OUT/tile/rust" --warmup "$WU" --iters "$IT" \
    --json "$RAW/tile_rust.json"
fi

if ! want grib; then echo "skip grib"; else
  for case in "grib_mrms_fixture|mrms-gz|fixture_mrms_small.grib2.gz" \
              "grib_hrrr_lambert_mini|grib-multi|fixture_hrrr_lambert_mini.grib2" \
              "grib_mrms_synth|mrms-gz|mrms_synth_3500x7000.grib2.gz"; do
    IFS='|' read -r label kind file <<<"$case"
    cwu=$GRIB_WU; cit=$GRIB_IT
    if [ "$label" = "grib_mrms_synth" ]; then cwu=$WU; cit=$IT; fi
    step "GRIB decode $label: Python"
    "$PY" "$BENCH/py/bench_grib.py" --kind "$kind" --file "$INPUTS/$file" --label "$label" \
      --warmup "$cwu" --iters "$cit" --json "$RAW/${label}_py.json"
    step "GRIB decode $label: Rust"
    "$RBIN/bench_grib" --kind "$kind" --file "$INPUTS/$file" --label "$label" \
      --warmup "$cwu" --iters "$cit" --json "$RAW/${label}_rust.json"
  done
fi

if ! want storms; then echo "skip storms"; else
  step "Storms: Python detect_storms + tracking"
  "$PY" "$BENCH/py/bench_storms.py" --warmup "$WU" --iters "$IT" --json "$RAW/storms_py.json"
  step "Storms: Rust radar-storms"
  "$RBIN/bench_storms" --input-dir "$INPUTS" --summary-out "$OUT/storms/rust_summary.json" \
    --warmup "$WU" --iters "$IT" --json "$RAW/storms_rust.json"
fi

if ! want nowcast; then echo "skip nowcast"; else
  step "Nowcast 875x1750: Python (production _run_nowcast)"
  "$PY" "$BENCH/py/bench_nowcast.py" --shape 875x1750 --warmup "$NOW_SMALL_WU" --iters "$NOW_SMALL_IT" \
    --json "$RAW/nowcast_875x1750_py.json"
  step "Nowcast 875x1750: Rust radar-nowcast"
  "$RBIN/bench_nowcast" --shape 875x1750 --input-dir "$INPUTS" --out-dir "$OUT/nowcast" \
    --warmup "$NOW_SMALL_WU" --iters "$NOW_SMALL_IT" --json "$RAW/nowcast_875x1750_rust.json"
  if [ "$NOW_PROD_IT" -gt 0 ]; then
    step "Nowcast 1750x3500 (production size): Python"
    "$PY" "$BENCH/py/bench_nowcast.py" --shape 1750x3500 --warmup "$NOW_PROD_WU" --iters "$NOW_PROD_IT" \
      --json "$RAW/nowcast_1750x3500_py.json"
    step "Nowcast 1750x3500 (production size): Rust"
    "$RBIN/bench_nowcast" --shape 1750x3500 --input-dir "$INPUTS" --out-dir "$OUT/nowcast" \
      --warmup "$NOW_PROD_WU" --iters "$NOW_PROD_IT" --json "$RAW/nowcast_1750x3500_rust.json"
  fi
fi

step "Cross-check outputs"
"$PY" "$BENCH/py/verify.py" > "$BENCH_WORK/verify.log" || { cat "$BENCH_WORK/verify.log"; exit 1; }

step "Write report"
"$PY" "$BENCH/py/report.py"

# The pyramid trees kept for verification are no longer needed.
rm -rf "$OUT"/tile/*/iter* 2>/dev/null || true
echo; echo "done. results in $BENCH/results/"
