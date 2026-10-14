#!/usr/bin/env bash
# radar-ng pre-cutover check for the Rust port. One command, run on a machine with network access
# and Docker (the owner's desktop):
#
#   rust/scripts/preflight.sh                 # everything; about 30-60 min on first run
#   rust/scripts/preflight.sh --dry-run       # print what would run
#   rust/scripts/preflight.sh --help
#
# Steps (each prints PASS/FAIL/SKIP; the run continues past failures and exits 1 if any failed):
#   1 tools     toolchain present, with Arch install hints
#   2 test      cargo test --workspace (includes the offline Temporal replay gate)
#   3 temporal  deploy/temporal-dev.yml (postgres + temporal), radar-worker integration tests, down
#   4 noaa      real MRMS + HRRR (+ NAQFC) files: Python (pygrib, production code) vs Rust decode,
#               selection, axes, S3 403/404 probe, then Python indexed render vs Rust render with
#               `radar parity --exact`. Checklist: rust/crates/radar-grib/README.md
#   5 docker    build deploy/rust/Dockerfile and Dockerfile.open-meteo, glibc floor (RUNBOOK 8.1),
#               image users (8.2), smoke run of `radar api` (/api/livez 200, /api/health 503)
#
# Logs and outputs: bench/work/preflight/<UTC timestamp>/ (summary.txt, one log per step).
# NOAA downloads are cached in bench/work/noaa/ and reused.
set -euo pipefail

# ------------------------------------------------------------------ options

usage() {
  cat <<'EOF'
Usage: rust/scripts/preflight.sh [options]

Step selection (steps: tools test temporal noaa docker):
  --only STEP[,STEP]     run only these steps (repeatable)
  --skip-test            skip cargo test --workspace
  --skip-temporal        skip the Temporal integration tests
  --skip-noaa            skip the real-NOAA comparison
  --skip-docker          skip the image builds and the smoke run
  --dry-run              print the commands each step would run, run nothing

Temporal:
  --keep-temporal        leave postgres + temporal running afterwards
  --temporal-address A   use an existing disposable server at A (host:port) instead of compose

NOAA:
  --noaa-cached          use the newest files already in bench/work/noaa (no network)
  --skip-naqfc           do not fetch/compare the NAQFC file (README items 9, 10)
  --mrms-prefix P        MRMS product (default CONUS/MergedBaseReflectivityQC_00.50)
  --local-mrms FILE      use this MRMS .grib2.gz instead of downloading (offline test)
  --local-hrrr FILE      use this HRRR .grib2 instead of downloading (offline test)
  --local-hrrr-idx FILE  .idx for --local-hrrr (default FILE.idx, generated when missing)

Build:
  --profile release|debug   cargo profile for the radar and noaa_dump binaries (default release)

Environment (all optional):
  PREFLIGHT_CARGO_TEST_ARGS  replaces "--workspace" in step 2 (e.g. "-p radar-grib")
  PREFLIGHT_PYTHON           Python 3.12 with bench/py/requirements.txt (default: bench/.venv, created)
  PREFLIGHT_ENV              shell file sourced before cargo (proxy settings, CARGO_TARGET_DIR)
  PREFLIGHT_LOG_DIR          where logs go (default bench/work/preflight/<timestamp>)
  PREFLIGHT_NOAA_CACHE       NOAA download cache (default bench/work/noaa)
  PREFLIGHT_TEMPORAL_TIMEOUT seconds to wait for Temporal (default 240)
  Layout overrides (default from the repository root):
  PREFLIGHT_RUST_DIR, PREFLIGHT_BENCH_DIR, PREFLIGHT_DEPLOY_DIR (deploy/rust),
  PREFLIGHT_TEMPORAL_COMPOSE (deploy/temporal-dev.yml), PREFLIGHT_DOCKER_CONTEXT (repo root),
  RADAR_NG (root of the Python tree that holds backend/, default repo root)
EOF
}

ALL_STEPS=(tools test temporal noaa docker)
declare -A WANT=([tools]=1 [test]=1 [temporal]=1 [noaa]=1 [docker]=1)
ONLY=()
DRY_RUN=0
KEEP_TEMPORAL=0
TEMPORAL_ADDR_OPT=""
NOAA_CACHED=0
WITH_NAQFC=1
MRMS_PREFIX="CONUS/MergedBaseReflectivityQC_00.50"
LOCAL_MRMS=""
LOCAL_HRRR=""
LOCAL_HRRR_IDX=""
PROFILE="release"

need_arg() { [ $# -ge 2 ] && [ -n "$2" ] || { echo "preflight: $1 needs a value" >&2; exit 2; }; }
while [ $# -gt 0 ]; do
  case "$1" in
    --only) need_arg "$@"; IFS=',' read -r -a parts <<<"$2"; ONLY+=("${parts[@]}"); shift ;;
    --skip-test) WANT[test]=0 ;;
    --skip-temporal) WANT[temporal]=0 ;;
    --skip-noaa) WANT[noaa]=0 ;;
    --skip-docker) WANT[docker]=0 ;;
    --dry-run) DRY_RUN=1 ;;
    --keep-temporal) KEEP_TEMPORAL=1 ;;
    --temporal-address) need_arg "$@"; TEMPORAL_ADDR_OPT="$2"; shift ;;
    --noaa-cached) NOAA_CACHED=1 ;;
    --skip-naqfc) WITH_NAQFC=0 ;;
    --mrms-prefix) need_arg "$@"; MRMS_PREFIX="$2"; shift ;;
    --local-mrms) need_arg "$@"; LOCAL_MRMS="$2"; shift ;;
    --local-hrrr) need_arg "$@"; LOCAL_HRRR="$2"; shift ;;
    --local-hrrr-idx) need_arg "$@"; LOCAL_HRRR_IDX="$2"; shift ;;
    --profile) need_arg "$@"; PROFILE="$2"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "preflight: unknown option $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done
case "$PROFILE" in release|debug) ;; *) echo "preflight: --profile must be release or debug" >&2; exit 2 ;; esac
if [ ${#ONLY[@]} -gt 0 ]; then
  for s in "${ALL_STEPS[@]}"; do WANT[$s]=0; done
  for s in "${ONLY[@]}"; do
    [ -n "${WANT[$s]+x}" ] || { echo "preflight: unknown step '$s' (steps: ${ALL_STEPS[*]})" >&2; exit 2; }
    WANT[$s]=1
  done
fi
if { [ -n "$LOCAL_MRMS" ] && [ -z "$LOCAL_HRRR" ]; } || { [ -z "$LOCAL_MRMS" ] && [ -n "$LOCAL_HRRR" ]; }; then
  echo "preflight: --local-mrms and --local-hrrr go together" >&2; exit 2
fi
LOCAL_MODE=0
[ -n "$LOCAL_MRMS" ] && LOCAL_MODE=1

# ------------------------------------------------------------------ layout

abspath() { (cd "$(dirname "$1")" && printf '%s/%s\n' "$(pwd)" "$(basename "$1")"); }
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO=""
if top="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null)" && [ -f "$top/rust/Cargo.toml" ]; then
  REPO="$top"
else
  REPO="$(cd "$SCRIPT_DIR/../.." && pwd)"
fi
RUST="${PREFLIGHT_RUST_DIR:-$REPO/rust}"
BENCH="${PREFLIGHT_BENCH_DIR:-$REPO/bench}"
DEPLOY="${PREFLIGHT_DEPLOY_DIR:-$REPO/deploy/rust}"
TEMPORAL_COMPOSE="${PREFLIGHT_TEMPORAL_COMPOSE:-$REPO/deploy/temporal-dev.yml}"
DOCKER_CONTEXT="${PREFLIGHT_DOCKER_CONTEXT:-$REPO}"
PY_ROOT="${RADAR_NG:-$REPO}"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
LOG_DIR="${PREFLIGHT_LOG_DIR:-$BENCH/work/preflight/$TS}"
NOAA_CACHE="${PREFLIGHT_NOAA_CACHE:-$BENCH/work/noaa}"
TEMPORAL_TIMEOUT="${PREFLIGHT_TEMPORAL_TIMEOUT:-240}"
VENV="$BENCH/.venv"
PY="${PREFLIGHT_PYTHON:-}"
[ -n "$LOCAL_MRMS" ] && LOCAL_MRMS="$(abspath "$LOCAL_MRMS")"
[ -n "$LOCAL_HRRR" ] && LOCAL_HRRR="$(abspath "$LOCAL_HRRR")"
[ -n "$LOCAL_HRRR_IDX" ] && LOCAL_HRRR_IDX="$(abspath "$LOCAL_HRRR_IDX")"

if [ "$DRY_RUN" = 0 ]; then mkdir -p "$LOG_DIR"; fi

if [ -n "${PREFLIGHT_ENV:-}" ]; then
  set +u
  # shellcheck disable=SC1090
  . "$PREFLIGHT_ENV"
  set -u
fi
TARGET_DIR="${CARGO_TARGET_DIR:-$RUST/target}"
if [ "$PROFILE" = release ]; then CARGO_PROFILE_FLAG=(--release); BIN_DIR="$TARGET_DIR/release"
else CARGO_PROFILE_FLAG=(); BIN_DIR="$TARGET_DIR/debug"; fi
RADAR_BIN="$BIN_DIR/radar"
NOAA_DUMP="$BIN_DIR/noaa_dump"

# ------------------------------------------------------------------ reporting

if [ -t 1 ]; then C_PASS=$'\e[32m'; C_FAIL=$'\e[31m'; C_WARN=$'\e[33m'; C_DIM=$'\e[2m'; C_OFF=$'\e[0m'
else C_PASS=""; C_FAIL=""; C_WARN=""; C_DIM=""; C_OFF=""; fi
ROWS=()          # "id|name|status|seconds|detail"
FAILED=0
STEP_NO=0
CUR_LOG=/dev/null

color() {
  case "$1" in
    PASS) printf '%s%s%s' "$C_PASS" "$1" "$C_OFF" ;;
    FAIL) printf '%s%s%s' "$C_FAIL" "$1" "$C_OFF" ;;
    WARN|SKIP|DRY|INFO) printf '%s%s%s' "$C_WARN" "$1" "$C_OFF" ;;
    *) printf '%s' "$1" ;;
  esac
}

# record ID NAME STATUS SECONDS DETAIL
record() {
  ROWS+=("$1|$2|$3|$4|$5")
  [ "$3" = FAIL ] && FAILED=1
  printf '  -> %s %s%s%s\n' "$(color "$3")" "$2" "${5:+: }" "$5"
}

header() {
  STEP_NO=$((STEP_NO + 1))
  printf '\n==== %d. %s ====\n' "$STEP_NO" "$1"
  if [ "$DRY_RUN" = 0 ]; then CUR_LOG="$LOG_DIR/$STEP_NO-$2.log"; : > "$CUR_LOG"; else CUR_LOG=/dev/null; fi
}

note() { printf '  %s\n' "$*"; [ "$CUR_LOG" = /dev/null ] || printf '%s\n' "$*" >> "$CUR_LOG"; }

quote_cmd() { local out="" a; for a in "$@"; do out+="$(printf '%q' "$a") "; done; printf '%s' "${out% }"; }

# run CMD...: print it, run it (output to the terminal and the step log), return its status.
# In --dry-run, print only.
run() {
  printf '  %s$ %s%s\n' "$C_DIM" "$(quote_cmd "$@")" "$C_OFF"
  [ "$DRY_RUN" = 1 ] && return 0
  printf '$ %s\n' "$(quote_cmd "$@")" >> "$CUR_LOG"
  local rc
  set +e
  "$@" 2>&1 | tee -a "$CUR_LOG"
  rc=${PIPESTATUS[0]}
  set -e
  return "$rc"
}

# run_in DIR CMD...: run in a directory.
run_in() {
  local dir="$1"; shift
  printf '  %s$ (cd %q && %s)%s\n' "$C_DIM" "$dir" "$(quote_cmd "$@")" "$C_OFF"
  [ "$DRY_RUN" = 1 ] && return 0
  printf '$ (cd %q && %s)\n' "$dir" "$(quote_cmd "$@")" >> "$CUR_LOG"
  local rc
  set +e
  (cd "$dir" && "$@") 2>&1 | tee -a "$CUR_LOG"
  rc=${PIPESTATUS[0]}
  set -e
  return "$rc"
}

# quiet CMD...: run without echoing, output only to the log (probes, inspections).
quiet() { if [ "$CUR_LOG" = /dev/null ]; then "$@" >/dev/null 2>&1; else "$@" >> "$CUR_LOG" 2>&1; fi; }

now_s() { date +%s; }

CLEANUP=()            # commands run on exit (smoke container)
TEMPORAL_DOWN=0       # 1 while this run owns a compose stack that must be stopped on exit
cleanup() {
  local c
  for c in "${CLEANUP[@]+"${CLEANUP[@]}"}"; do eval "$c" >/dev/null 2>&1 || true; done
  if [ "$TEMPORAL_DOWN" = 1 ]; then docker compose -f "$TEMPORAL_COMPOSE" down >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT
trap 'echo; echo "preflight: interrupted"; exit 130' INT TERM

have() { command -v "$1" >/dev/null 2>&1; }

# ------------------------------------------------------------------ step 1: tools

TOOL_MISSING=()
tool_row() { # tool_row NAME STATUS DETAIL
  local name="$1" status="$2" detail="$3"
  printf '  %-22s %s  %s\n' "$name" "$(color "$status")" "$detail"
  [ "$CUR_LOG" = /dev/null ] || printf '%-22s %s  %s\n' "$name" "$status" "$detail" >> "$CUR_LOG"
  [ "$status" = FAIL ] && TOOL_MISSING+=("$name")
  return 0
}

python_for_bench() {
  # Echo the interpreter to create the venv from (uv preferred), or nothing.
  if have uv; then echo "uv"; elif have python3.12; then echo "python3.12"; fi
}

step_tools() {
  header "Tools" tools
  local t0; t0=$(now_s)
  TOOL_MISSING=()
  local need_cargo=$(( WANT[test] + WANT[temporal] + WANT[noaa] ))
  local need_docker=$(( WANT[temporal] + WANT[docker] ))
  [ -n "$TEMPORAL_ADDR_OPT" ] && need_docker=${WANT[docker]}

  if have cargo && have rustc; then
    local v; v="$(rustc --version | awk '{print $2}')"
    if [ "$(printf '%s\n1.92.0\n' "$v" | sort -V | head -1)" = "1.92.0" ]; then tool_row "rustc/cargo" PASS "$v"
    else tool_row "rustc/cargo" FAIL "$v < 1.92: rustup update stable"; fi
  elif [ "$need_cargo" -gt 0 ]; then tool_row "rustc/cargo" FAIL "sudo pacman -S rustup && rustup default stable"
  else tool_row "rustc/cargo" SKIP "not needed for the selected steps"; fi

  if [ -n "${PROTOC:-}" ] && [ -x "${PROTOC}" ]; then tool_row protoc PASS "PROTOC=$PROTOC"
  elif have protoc; then tool_row protoc PASS "$(protoc --version)"
  elif [ "$need_cargo" -gt 0 ]; then tool_row protoc FAIL "sudo pacman -S protobuf (temporalio's proto build)"
  else tool_row protoc SKIP "not needed"; fi

  if have curl; then tool_row curl PASS "$(curl --version | head -1 | awk '{print $1" "$2}')"
  else tool_row curl FAIL "sudo pacman -S curl"; fi

  if [ "$need_docker" -gt 0 ]; then
    if ! have docker; then
      tool_row docker FAIL "sudo pacman -S docker docker-compose docker-buildx && sudo systemctl enable --now docker.service && sudo usermod -aG docker \$USER (log out and in)"
    else
      tool_row docker PASS "$(docker --version)"
      if docker compose version >/dev/null 2>&1; then tool_row "docker compose" PASS "$(docker compose version --short 2>/dev/null)"
      else tool_row "docker compose" FAIL "sudo pacman -S docker-compose"; fi
      if [ "${WANT[docker]}" = 1 ]; then
        if docker buildx version >/dev/null 2>&1; then tool_row "docker buildx" PASS "$(docker buildx version | awk '{print $2}')"
        else tool_row "docker buildx" FAIL "sudo pacman -S docker-buildx (the Dockerfile uses BuildKit cache mounts)"; fi
      fi
      if docker info >/dev/null 2>&1; then tool_row "docker daemon" PASS "reachable"
      else tool_row "docker daemon" FAIL "not reachable: sudo systemctl start docker.service; user in the docker group?"; fi
    fi
  else tool_row docker SKIP "not needed for the selected steps"; fi

  if [ "${WANT[noaa]}" = 1 ]; then
    if [ -n "$PY" ]; then tool_row python PASS "PREFLIGHT_PYTHON=$PY"
    elif [ -x "$VENV/bin/python" ]; then tool_row python PASS "$VENV ($("$VENV/bin/python" --version 2>&1))"
    else
      case "$(python_for_bench)" in
        uv) tool_row "uv (Python 3.12)" PASS "$(uv --version); creates $VENV with Python 3.12" ;;
        python3.12) tool_row python3.12 PASS "$(python3.12 --version); uv not found, using venv + pip" ;;
        *) tool_row "uv / python3.12" FAIL "sudo pacman -S uv (the bench pins numpy<2 and pygrib 2.1.8, which need Python 3.12; uv fetches it)" ;;
      esac
    fi
  else tool_row "uv / python3.12" SKIP "not needed"; fi

  if have git; then tool_row git PASS "$(git --version | awk '{print $3}')"; else tool_row git SKIP "optional"; fi
  if have objdump; then tool_row objdump PASS "glibc floor check"; else tool_row objdump SKIP "optional (binutils); the glibc check falls back to grep"; fi

  # The tool check is read-only, so it runs in --dry-run too.
  if [ ${#TOOL_MISSING[@]} -eq 0 ]; then record 1 tools PASS "$(( $(now_s) - t0 ))" "all required tools present"
  else record 1 tools FAIL "$(( $(now_s) - t0 ))" "missing: ${TOOL_MISSING[*]}"; fi
}

# ------------------------------------------------------------------ step 2: cargo test

step_test() {
  header "cargo test (workspace, offline replay gate)" test
  local t0; t0=$(now_s)
  local -a args
  # shellcheck disable=SC2206
  if [ -n "${PREFLIGHT_CARGO_TEST_ARGS:-}" ]; then args=($PREFLIGHT_CARGO_TEST_ARGS); else args=(--workspace); fi
  if ! run_in "$RUST" cargo test "${args[@]}"; then
    local nfail
    nfail="$(grep -Eo '[0-9]+ failed' "$CUR_LOG" | awk '{s+=$1} END {print s+0}' || true)"
    record 2 "cargo test ${args[*]}" FAIL "$(( $(now_s) - t0 ))" "$nfail failed (a build error counts 0); see $CUR_LOG"
    return 0
  fi
  if [ "$DRY_RUN" = 1 ]; then record 2 "cargo test ${args[*]}" DRY 0 ""; return 0; fi
  local passed failed ignored
  passed=$(grep -Eo '^test result: [a-zA-Z]+\. [0-9]+ passed' "$CUR_LOG" | awk '{s+=$4} END {print s+0}')
  failed=$(grep -Eo '[0-9]+ failed' "$CUR_LOG" | awk '{s+=$1} END {print s+0}')
  ignored=$(grep -Eo '[0-9]+ ignored' "$CUR_LOG" | awk '{s+=$1} END {print s+0}')
  local replay="replay gate not in this run"
  grep -q 'Running tests/replay.rs' "$CUR_LOG" && replay="replay gate ran"
  record 2 "cargo test ${args[*]}" PASS "$(( $(now_s) - t0 ))" "$passed passed, $failed failed, $ignored ignored; $replay"
}

# ------------------------------------------------------------------ step 3: temporal

port_open() { (exec 3<>"/dev/tcp/$1/$2") 2>/dev/null; }

namespace_ready() {
  # Prefer a CLI that can describe the namespace; fall back to the auto-setup log.
  if have temporal; then
    temporal operator namespace describe --namespace default --address "127.0.0.1:7233" >/dev/null 2>&1 && return 0
  fi
  docker exec radar-ng-temporal sh -c \
    'a="${TEMPORAL_ADDRESS:-$(hostname -i | awk "{print \$1}"):7233}"; temporal operator namespace describe --namespace default --address "$a" || tctl --address "$a" --ns default namespace describe' \
    >/dev/null 2>&1 && return 0
  return 1
}

step_temporal() {
  header "Temporal integration tests (deploy/temporal-dev.yml)" temporal
  local t0; t0=$(now_s)
  local addr="${TEMPORAL_ADDR_OPT:-localhost:7233}"
  local started=0
  if [ -z "$TEMPORAL_ADDR_OPT" ]; then
    if [ "$DRY_RUN" = 0 ] && [ ! -f "$TEMPORAL_COMPOSE" ]; then
      record 3 temporal FAIL 0 "$TEMPORAL_COMPOSE not found"; return 0
    fi
    local already=0
    if [ "$DRY_RUN" = 0 ] && docker ps --format '{{.Names}}' 2>/dev/null | grep -qx radar-ng-temporal; then
      already=1; note "radar-ng-temporal is already running; it is reused and left running"
    elif [ "$DRY_RUN" = 0 ] && port_open 127.0.0.1 7233; then
      record 3 temporal FAIL 0 "something else listens on :7233; stop it, or pass --temporal-address HOST:PORT (a disposable server: the seeding test writes schedules)"
      return 0
    fi
    if [ "$already" = 0 ]; then
      if ! run docker compose -f "$TEMPORAL_COMPOSE" up -d postgres temporal; then
        record 3 "temporal up" FAIL "$(( $(now_s) - t0 ))" "docker compose up failed (see $CUR_LOG)"
        return 0
      fi
      started=1
      [ "$KEEP_TEMPORAL" = 0 ] && [ "$DRY_RUN" = 0 ] && TEMPORAL_DOWN=1
    fi
    if [ "$DRY_RUN" = 1 ]; then
      note "wait until :7233 accepts connections and namespace 'default' describes (timeout ${TEMPORAL_TIMEOUT}s)"
    else
      local deadline=$(( $(now_s) + TEMPORAL_TIMEOUT )) ready=0 how="" open_since=""
      note "waiting for :7233 and namespace 'default' (timeout ${TEMPORAL_TIMEOUT}s)"
      while [ "$(now_s)" -lt "$deadline" ]; do
        if port_open 127.0.0.1 7233; then
          [ -n "$open_since" ] || open_since=$(now_s)
          if namespace_ready; then ready=1; how="namespace described"; break; fi
          if docker logs radar-ng-temporal 2>&1 | grep -Eiq 'namespace .*(registration complete|already registered)'; then
            ready=1; how="auto-setup log reports the namespace"; break
          fi
          # No CLI in reach and no recognisable log line: after 90 s of an open port, let the
          # integration tests decide (they fail clearly if the namespace is missing).
          if [ $(( $(now_s) - open_since )) -ge 90 ]; then
            ready=1; how="port open for 90 s; namespace not confirmed by a CLI or the log"; break
          fi
        fi
        sleep 3
      done
      if [ "$ready" = 0 ]; then
        quiet docker compose -f "$TEMPORAL_COMPOSE" logs --tail 80 temporal || true
        record 3 "temporal ready" FAIL "$(( $(now_s) - t0 ))" "not ready after ${TEMPORAL_TIMEOUT}s (logs in $CUR_LOG)"
        if [ "$started" = 1 ] && [ "$KEEP_TEMPORAL" = 0 ]; then
          run docker compose -f "$TEMPORAL_COMPOSE" down || true
          TEMPORAL_DOWN=0
        fi
        return 0
      fi
      note "temporal ready after $(( $(now_s) - t0 ))s ($how)"
    fi
  fi

  local t1; t1=$(now_s)
  local rc=0
  run_in "$RUST" env TEMPORAL_TEST_ADDRESS="$addr" TEMPORAL_TEST_NAMESPACE=default \
    cargo test -p radar-worker --test integration -- --test-threads=1 || rc=$?
  local status=PASS detail=""
  if [ "$DRY_RUN" = 1 ]; then status=DRY
  elif [ "$rc" != 0 ]; then
    status=FAIL
    detail="$(grep -Eo '[0-9]+ passed; [0-9]+ failed' "$CUR_LOG" | tail -1 || true); see $CUR_LOG"
  elif grep -q 'skipped: TEMPORAL_TEST_ADDRESS' "$CUR_LOG"; then status=FAIL; detail="tests skipped themselves (address not seen)"
  else detail="$(grep -Eo '^test result: ok\. [0-9]+ passed' "$CUR_LOG" | tail -1 | awk '{print $4" passed"}') against $addr"
  fi
  record 3 "radar-worker integration" "$status" "$(( $(now_s) - t1 ))" "$detail"

  if [ "$started" = 1 ] && [ "$KEEP_TEMPORAL" = 0 ]; then
    run docker compose -f "$TEMPORAL_COMPOSE" down || note "compose down failed"
    TEMPORAL_DOWN=0
  elif [ "$started" = 1 ]; then
    note "--keep-temporal: left running; stop with: docker compose -f $TEMPORAL_COMPOSE down"
  fi
}

# ------------------------------------------------------------------ step 4: noaa

ensure_python() {
  # Sets PY to a Python 3.12 with bench/py/requirements.txt; returns 1 with a hint on failure.
  local req="$BENCH/py/requirements.txt"
  if [ -n "$PY" ]; then return 0; fi
  PY="$VENV/bin/python"
  local stamp="$VENV/.preflight-requirements.sha256" want
  if [ "$DRY_RUN" = 1 ]; then
    if [ "$(python_for_bench)" = uv ] || [ "$(python_for_bench)" = "" ]; then
      run uv venv --python 3.12 "$VENV"
      run uv pip install --python "$PY" -r "$req"
    else
      run python3.12 -m venv "$VENV"
      run "$PY" -m pip install -r "$req"
    fi
    return 0
  fi
  want="$(sha256sum "$req" | awk '{print $1}')"
  if [ -x "$PY" ] && [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$want" ] \
     && "$PY" -c 'import sys, pygrib, numpy; sys.exit(sys.version_info[:2] != (3, 12))' >/dev/null 2>&1; then
    note "reusing $VENV ($("$PY" --version 2>&1))"
    return 0
  fi
  case "$(python_for_bench)" in
    uv)
      run uv venv --allow-existing --python 3.12 "$VENV" || return 1
      run uv pip install --python "$PY" -r "$req" || return 1 ;;
    python3.12)
      run python3.12 -m venv "$VENV" || return 1
      run "$PY" -m pip install -r "$req" || return 1 ;;
    *) note "no uv and no python3.12: sudo pacman -S uv"; return 1 ;;
  esac
  printf '%s\n' "$want" > "$stamp"
}

# Keep the cache bounded: the newest 3 MRMS files, 2 HRRR runs and 2 NAQFC files (by modification time).
prune_cache() {
  local dir keep entry
  for spec in "mrms:3" "hrrr:2" "naqfc:2"; do
    dir="$NOAA_CACHE/${spec%%:*}"; keep="${spec##*:}"
    [ -d "$dir" ] || continue
    while IFS= read -r entry; do
      [ -n "$entry" ] && rm -rf -- "${dir:?}/$entry"
    done < <(find "$dir" -mindepth 1 -maxdepth 1 ! -name '*.part' -printf '%T@ %f\n' | sort -n | head -n "-$keep" | cut -d' ' -f2-)
  done
}

# json_get FILE EXPR: evaluate a Python expression over the parsed JSON document `d`.
json_get() { "$PY" -c 'import json,sys; d=json.load(open(sys.argv[1])); v=eval(sys.argv[2]); print("" if v is None else v)' "$1" "$2"; }

step_noaa() {
  header "Real NOAA files: Python vs Rust (radar-grib README checklist)" noaa
  local t0; t0=$(now_s)
  local work="$LOG_DIR/noaa"
  [ "$DRY_RUN" = 0 ] && mkdir -p "$work"
  [ "$DRY_RUN" = 0 ] && [ "$LOCAL_MODE" = 0 ] && mkdir -p "$NOAA_CACHE"
  if [ "$DRY_RUN" = 0 ] && [ ! -d "$PY_ROOT/backend" ]; then
    record 4 "noaa setup" FAIL 0 "$PY_ROOT/backend not found (set RADAR_NG to the repo root)"; return 0
  fi

  # 4a: Python env and the two Rust binaries.
  if ! ensure_python; then
    record 4a "python 3.12 env" FAIL "$(( $(now_s) - t0 ))" "could not create $VENV (see $CUR_LOG)"; return 0
  fi
  if ! run_in "$RUST" cargo build "${CARGO_PROFILE_FLAG[@]}" -p radar --bin radar -p radar-bench --bin noaa_dump; then
    record 4a "build radar + noaa_dump" FAIL "$(( $(now_s) - t0 ))" "see $CUR_LOG"; return 0
  fi
  record 4a "setup (venv, build $PROFILE)" "$([ "$DRY_RUN" = 1 ] && echo DRY || echo PASS)" "$(( $(now_s) - t0 ))" "python $PY"

  # 4b: inputs (download with the radar-grib network code, or local files).
  local t1; t1=$(now_s)
  local mrms hrrr idx subset="" naqfc="" fetch_json=""
  local -a pyenv=(env "RADAR_NG=$PY_ROOT" PYTHONDONTWRITEBYTECODE=1 "BENCH_WORK=$BENCH/work")
  if [ "$LOCAL_MODE" = 1 ]; then
    mrms="$LOCAL_MRMS"; hrrr="$LOCAL_HRRR"; idx="${LOCAL_HRRR_IDX:-$LOCAL_HRRR.idx}"
    if [ "$DRY_RUN" = 0 ] && [ ! -f "$idx" ]; then
      idx="$work/$(basename "$hrrr").idx"
      run "${pyenv[@]}" "$PY" "$BENCH/py/verify_noaa.py" make-idx --grib "$hrrr" --out "$idx" || true
    fi
    record 4b "inputs (local files)" "$([ "$DRY_RUN" = 1 ] && echo DRY || echo SKIP)" 0 \
      "no download, no S3 status probe: $(basename "$mrms"), $(basename "$hrrr")"
  else
    fetch_json="$work/fetch.json"
    local -a fetch=(env RUST_BACKTRACE=0 "$NOAA_DUMP" fetch --cache "$NOAA_CACHE" --mrms-prefix "$MRMS_PREFIX" --json "$fetch_json")
    [ "$NOAA_CACHED" = 1 ] && fetch+=(--cached-only)
    [ "$WITH_NAQFC" = 1 ] && fetch+=(--naqfc)
    if ! run "${fetch[@]}"; then
      record 4b "fetch NOAA files" FAIL "$(( $(now_s) - t1 ))" "noaa_dump fetch failed (network? see $CUR_LOG); retry, or --noaa-cached"
      return 0
    fi
    if [ "$DRY_RUN" = 1 ]; then
      mrms="<newest MRMS .grib2.gz>"; hrrr="<hrrr wrfsfcf01.grib2>"; idx="$hrrr.idx"
      subset="<subset_f01.grib2>"; [ "$WITH_NAQFC" = 1 ] && naqfc="<naqfc .grib2>"
    else
      mrms="$(json_get "$fetch_json" 'd["mrms"]["file"]' || true)"
      hrrr="$(json_get "$fetch_json" 'd["hrrr"]["file"]' || true)"
      idx="$(json_get "$fetch_json" 'd["hrrr"]["idx_file"]' || true)"
      subset="$(json_get "$fetch_json" 'd["hrrr"].get("subset_file")' || true)"
      [ "$WITH_NAQFC" = 1 ] && naqfc="$(json_get "$fetch_json" '(d.get("naqfc") or {}).get("file")' || true)"
      if [ -z "$mrms" ] || [ -z "$hrrr" ] || [ -z "$idx" ]; then
        record 4b "fetch NOAA files" FAIL "$(( $(now_s) - t1 ))" "fetch report $fetch_json lacks file paths"
        return 0
      fi
    fi
    [ "$DRY_RUN" = 0 ] && prune_cache
    record 4b "fetch NOAA files" "$([ "$DRY_RUN" = 1 ] && echo DRY || echo PASS)" "$(( $(now_s) - t1 ))" \
      "$(basename "$mrms"), $(basename "$(dirname "$hrrr")")/$(basename "$hrrr")$([ -n "$naqfc" ] && echo ", NAQFC $(basename "$naqfc")")"
  fi

  # 4c: Rust dump, then the Python comparison (items 1-10).
  local t2; t2=$(now_s)
  local -a dump=(env RUST_BACKTRACE=0 "$NOAA_DUMP" dump --mrms "$mrms" --hrrr "$hrrr" --hrrr-idx "$idx" --out "$work/rust")
  [ -n "$subset" ] && dump+=(--hrrr-subset "$subset")
  [ -n "$naqfc" ] && dump+=(--naqfc "$naqfc")
  local -a cmp=("${pyenv[@]}" "$PY" "$BENCH/py/verify_noaa.py" compare --rust-dir "$work/rust"
                --mrms "$mrms" --hrrr "$hrrr" --hrrr-idx "$idx" --work "$work/py-tmp" --json "$work/verify.json")
  [ -n "$fetch_json" ] && cmp+=(--fetch-json "$fetch_json")
  [ -n "$naqfc" ] && cmp+=(--naqfc "$naqfc")
  [ "$LOCAL_MODE" = 1 ] && cmp+=(--allow-missing-vars)
  if ! run "${dump[@]}"; then
    record 4c "rust decode dump" FAIL "$(( $(now_s) - t2 ))" "noaa_dump dump failed (see $CUR_LOG)"
  else
    local rc=0
    run "${cmp[@]}" || rc=$?
    if [ "$DRY_RUN" = 1 ]; then record 4c "decode compare (items 1-10)" DRY 0 ""
    else
      local counts
      counts="$(json_get "$work/verify.json" '", ".join("%s %d" % (s, sum(r["status"] == s for r in d["rows"])) for s in ("PASS", "FAIL", "WARN", "SKIP"))' || true)"
      record 4c "decode compare (items 1-10)" "$([ "$rc" = 0 ] && echo PASS || echo FAIL)" "$(( $(now_s) - t2 ))" \
        "$counts; table in $CUR_LOG, rows in $work/verify.json"
      local probes
      probes="$(json_get "$work/verify.json" '"; ".join("%s: HEAD %s" % kv for kv in (d["notes"].get("missing_object_status") or {}).items())' || true)"
      if [ -n "$probes" ]; then
        MISSING_KEY_STATUS="$probes"
        record 4c "S3 missing-key status (item 1)" INFO 0 "$probes"
      fi
    fi
  fi

  # 4d: Python indexed render vs Rust render of the MRMS frame, radar parity --exact.
  local t3; t3=$(now_s)
  local py_tiles="$work/tiles-py" rs_tiles="$work/tiles-rs"
  [ "$DRY_RUN" = 0 ] && rm -rf "$py_tiles" "$rs_tiles"
  if ! run "${pyenv[@]}" "$PY" "$BENCH/py/verify_noaa.py" render --mrms "$mrms" --out "$py_tiles" --work "$work/py-tmp"; then
    record 4d "render parity" FAIL "$(( $(now_s) - t3 ))" "python indexed render failed (see $CUR_LOG)"; return 0
  fi
  if ! run env LOG_LEVEL=warn "$RADAR_BIN" render-once "$mrms" "$rs_tiles/radar"; then
    record 4d "render parity" FAIL "$(( $(now_s) - t3 ))" "radar render-once failed (see $CUR_LOG)"; return 0
  fi
  local prc=0
  run "$RADAR_BIN" parity --python-root "$py_tiles" --rust-root "$rs_tiles" --exact || prc=$?
  if [ "$DRY_RUN" = 1 ]; then record 4d "render parity --exact" DRY 0 ""
  elif [ "$prc" = 0 ]; then
    record 4d "render parity --exact" PASS "$(( $(now_s) - t3 ))" "$(find "$rs_tiles" -name '*.png' | wc -l) Rust tiles, identical to Python indexed"
  else
    record 4d "render parity --exact" FAIL "$(( $(now_s) - t3 ))" "radar parity exit $prc (see $CUR_LOG)"
  fi
}

# ------------------------------------------------------------------ step 5: docker

max_glibc_of() { # max GLIBC_x.y symbol version a binary needs
  if have objdump; then objdump -T "$1" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1
  else grep -aoE 'GLIBC_[0-9]+\.[0-9]+(\.[0-9]+)?' "$1" | sed 's/GLIBC_//' | sort -uV | tail -1; fi
}

step_docker() {
  header "Docker images, glibc floor, image users, smoke run" docker
  local t0; t0=$(now_s)
  local radar_tag=radar-ng-radar:preflight om_tag=radar-ng-open-meteo-worker:preflight
  if [ "$DRY_RUN" = 0 ]; then
    if [ ! -f "$DEPLOY/Dockerfile" ]; then record 5 docker FAIL 0 "$DEPLOY/Dockerfile not found"; return 0; fi
    local styles="$DOCKER_CONTEXT/backend/basemap/styles"
    if [ ! -d "$styles" ] || [ -z "$(ls -A "$styles" 2>/dev/null)" ]; then
      record 5a "build radar image" FAIL 0 "$styles is missing or empty: the image copies it (deploy/rust/README 'Known problems'); restore it from the Python api image or git history"
      return 0
    fi
    if ! docker info >/dev/null 2>&1; then record 5 docker FAIL 0 "Docker daemon not reachable"; return 0; fi
  fi

  # 5a: builds (context = repo root; the radar Dockerfile has its own .dockerignore next to it).
  if ! run env DOCKER_BUILDKIT=1 docker build -f "$DEPLOY/Dockerfile" -t "$radar_tag" "$DOCKER_CONTEXT"; then
    record 5a "build radar image" FAIL "$(( $(now_s) - t0 ))" "see $CUR_LOG"; return 0
  fi
  record 5a "build radar image" "$([ "$DRY_RUN" = 1 ] && echo DRY || echo PASS)" "$(( $(now_s) - t0 ))" ""
  local t1; t1=$(now_s)
  if ! run env DOCKER_BUILDKIT=1 docker build -f "$DEPLOY/Dockerfile.open-meteo" \
       --build-arg "RADAR_IMAGE=$radar_tag" -t "$om_tag" "$DOCKER_CONTEXT"; then
    record 5a "build open-meteo image" FAIL "$(( $(now_s) - t1 ))" "see $CUR_LOG"; return 0
  fi
  record 5a "build open-meteo image" "$([ "$DRY_RUN" = 1 ] && echo DRY || echo PASS)" "$(( $(now_s) - t1 ))" ""

  # 5b: RUNBOOK 8.1 glibc floor.
  local t2; t2=$(now_s)
  if [ "$DRY_RUN" = 1 ]; then
    run docker create --name radar-ng-preflight-cp "$radar_tag"
    run docker cp radar-ng-preflight-cp:/usr/local/bin/radar "$LOG_DIR/radar-check"
    run docker rm radar-ng-preflight-cp
    note "max GLIBC_ symbol of radar-check (objdump -T) vs: docker run --rm --entrypoint /bin/sh $om_tag -c 'ldd --version | head -1'"
    run docker run --rm "$om_tag" --version
    record 5b "glibc floor (RUNBOOK 8.1)" DRY 0 ""
  else
    local bin="$LOG_DIR/radar-check" need have_v="" banner
    quiet docker rm -f radar-ng-preflight-cp || true
    quiet docker create --name radar-ng-preflight-cp "$radar_tag" \
      && quiet docker cp radar-ng-preflight-cp:/usr/local/bin/radar "$bin" || true
    quiet docker rm -f radar-ng-preflight-cp || true
    need="$( [ -f "$bin" ] && max_glibc_of "$bin" || true)"
    banner="$(docker run --rm --entrypoint /bin/sh "$om_tag" -c 'ldd --version 2>&1 | head -1' 2>/dev/null || true)"
    have_v="$(printf '%s' "$banner" | grep -oE '[0-9]+\.[0-9]+' | tail -1 || true)"
    if [ -z "$have_v" ]; then # no shell in the image: read the libc banner instead
      quiet docker rm -f radar-ng-preflight-libc || true
      if quiet docker create --name radar-ng-preflight-libc "$om_tag" \
         && quiet docker cp -L radar-ng-preflight-libc:/lib/x86_64-linux-gnu/libc.so.6 "$LOG_DIR/libc.so.6"; then
        have_v="$(grep -aoE 'release version [0-9]+\.[0-9]+' "$LOG_DIR/libc.so.6" | grep -oE '[0-9]+\.[0-9]+' | head -1 || true)"
      fi
      quiet docker rm -f radar-ng-preflight-libc || true
    fi
    local runs=0
    run docker run --rm "$om_tag" --version && runs=1
    note "radar needs GLIBC $need; open-meteo base has glibc ${have_v:-unknown}; radar --version in the open-meteo image: $([ "$runs" = 1 ] && echo ok || echo FAILED)"
    if [ -n "$need" ] && [ -n "$have_v" ] && [ "$(printf '%s\n%s\n' "$need" "$have_v" | sort -V | tail -1)" = "$have_v" ] && [ "$runs" = 1 ]; then
      record 5b "glibc floor (RUNBOOK 8.1)" PASS "$(( $(now_s) - t2 ))" "radar needs $need, base has $have_v, binary runs"
    elif [ "$runs" = 1 ] && [ -z "$have_v" ]; then
      record 5b "glibc floor (RUNBOOK 8.1)" WARN "$(( $(now_s) - t2 ))" "base glibc unknown; radar needs ${need:-?} and runs in the image"
    else
      record 5b "glibc floor (RUNBOOK 8.1)" FAIL "$(( $(now_s) - t2 ))" "radar needs ${need:-?}, base has ${have_v:-?}, runs=$runs"
    fi
    rm -f "$bin" "$LOG_DIR/libc.so.6"
  fi

  # 5c: RUNBOOK 8.2 image users.
  if [ "$DRY_RUN" = 1 ]; then
    run docker image inspect "$radar_tag" --format '{{.Config.User}}'
    run docker image inspect "$om_tag" --format '{{.Config.User}}'
    record 5c "image users (RUNBOOK 8.2)" DRY 0 ""
  else
    local u1 u2
    u1="$(docker image inspect "$radar_tag" --format '{{.Config.User}}' 2>/dev/null || true)"
    u2="$(docker image inspect "$om_tag" --format '{{.Config.User}}' 2>/dev/null || true)"
    if [ "$u1" = "1000:1000" ] && [ "$u2" = "999:999" ]; then record 5c "image users (RUNBOOK 8.2)" PASS 0 "radar $u1, open-meteo $u2"
    else record 5c "image users (RUNBOOK 8.2)" FAIL 0 "radar '$u1' (want 1000:1000), open-meteo '$u2' (want 999:999)"; fi
  fi

  # 5d: smoke: radar api with empty TILE_DIR/STATE_DIR/GRID_DIR on a random port.
  local t3; t3=$(now_s)
  local name="radar-ng-preflight-api-$$" data="$LOG_DIR/smoke-data"
  local -a drun=(docker run -d --name "$name" -p 127.0.0.1::8080
                 -v "$data/tiles:/data/tiles:ro" -v "$data/grids:/data/grids:ro" -v "$data/state:/data/state:ro"
                 "$radar_tag")
  if [ "$DRY_RUN" = 1 ]; then
    run "${drun[@]}"
    note "curl http://127.0.0.1:<port>/api/livez -> 200 {\"status\":\"ok\"}; /api/health -> 503 degraded (no_mrms_tiles)"
    run docker rm -f "$name"
    record 5d "smoke radar api" DRY 0 ""
    return 0
  fi
  mkdir -p "$data/tiles" "$data/grids" "$data/state"
  chmod 0755 "$data" "$data"/*
  CLEANUP+=("docker rm -f $name")
  if ! run "${drun[@]}"; then record 5d "smoke radar api" FAIL "$(( $(now_s) - t3 ))" "docker run failed"; return 0; fi
  local port="" code="" body="" hcode=""
  for _ in $(seq 1 30); do
    port="$(docker port "$name" 8080/tcp 2>/dev/null | head -1 | sed 's/.*://')"
    if [ -n "$port" ]; then
      code="$(curl --noproxy '*' -s -o "$LOG_DIR/livez.json" -w '%{http_code}' "http://127.0.0.1:$port/api/livez" || true)"
      [ "$code" = 200 ] && break
    fi
    sleep 1
  done
  body="$(cat "$LOG_DIR/livez.json" 2>/dev/null || true)"
  hcode="$(curl --noproxy '*' -s -o "$LOG_DIR/health.json" -w '%{http_code}' "http://127.0.0.1:${port:-0}/api/health" || true)"
  note "livez: $code $body"
  note "health: $hcode $(head -c 200 "$LOG_DIR/health.json" 2>/dev/null || true)"
  quiet docker logs "$name" || true
  quiet docker rm -f "$name" || true
  if [ "$code" = 200 ] && printf '%s' "$body" | grep -q '"status":"ok"' && [ "$hcode" = 503 ] \
     && grep -q 'no_mrms_tiles' "$LOG_DIR/health.json"; then
    record 5d "smoke radar api" PASS "$(( $(now_s) - t3 ))" "/api/livez 200 ok, /api/health 503 degraded (no_mrms_tiles), as documented for empty data"
  else
    record 5d "smoke radar api" FAIL "$(( $(now_s) - t3 ))" "/api/livez $code (want 200), /api/health $hcode (want 503 no_mrms_tiles); container log in $CUR_LOG"
  fi
}

# ------------------------------------------------------------------ main

MISSING_KEY_STATUS=""
printf 'radar-ng pre-cutover check  %s\n' "$TS"
printf '  repo %s\n  rust %s\n  bench %s\n  deploy %s\n  python tree %s\n' "$REPO" "$RUST" "$BENCH" "$DEPLOY" "$PY_ROOT"
[ "$DRY_RUN" = 1 ] && printf '  DRY RUN: commands are printed, not run\n' || printf '  logs %s\n' "$LOG_DIR"
START=$(now_s)

for s in "${ALL_STEPS[@]}"; do
  if [ "${WANT[$s]}" = 1 ]; then
    # Called in a condition, so set -e does not abort the run inside a step: each step checks its
    # commands and records PASS/FAIL itself. A nonzero return is an unexpected error in the step.
    if ! "step_$s"; then record "$STEP_NO" "$s" FAIL 0 "step stopped on an unexpected error (see $CUR_LOG)"; fi
  else
    STEP_NO=$((STEP_NO + 1))
    printf '\n==== %d. %s ==== %s\n' "$STEP_NO" "$s" "$(color SKIP)"
    ROWS+=("$STEP_NO|$s|SKIP|0|skipped by option")
  fi
done

summary() {
  printf '\n==== Summary (%ss) ====\n' "$(( $(now_s) - START ))"
  printf '  %-4s %-34s %-5s %6s  %s\n' step check status secs detail
  local row id name status secs detail
  for row in "${ROWS[@]}"; do
    IFS='|' read -r id name status secs detail <<<"$row"
    printf '  %-4s %-34s %-5s %6s  %s\n' "$id" "$name" "$status" "$secs" "$detail"
  done
  if [ -n "$MISSING_KEY_STATUS" ]; then
    printf '\n  *** S3 status for a missing key (radar-grib README item 1): %s\n' "$MISSING_KEY_STATUS"
    printf '  *** 404 everywhere: nothing to change. 403: map 403 to "not found" for those buckets before cutover\n'
    printf '  ***   (hrrr::fetch_hour pending path, .idx fallback, naqfc::download_product).\n'
  fi
}
summary_text="$(summary)"
printf '%s\n' "$summary_text" | sed "s/ PASS / $(color PASS) /; s/ FAIL / $(color FAIL) /"
[ "$DRY_RUN" = 0 ] && printf '%s\n' "$summary_text" > "$LOG_DIR/summary.txt"

cat <<EOF

Next steps
  - Fix every FAIL above and rerun (cached NOAA files are reused; --only <step> reruns one step).
  - Not covered here (radar-grib README): items 11 (listing encoding beyond the pagination this run
    used), 13 (00:00 UTC yesterday fallback), 14 (timeouts on the deployment network).
  - RUNBOOK: $DEPLOY/RUNBOOK.md section 2 (open items that block the cutover), section 4 (cluster
    pre-flight: baseline \`radar debug doctor\`, Worker Deployments, watch queue, storage, smoke),
    then section 5 (cutover). Run the k6 load test against the cluster (bench/README.md, load/K6_CLUSTER.md).
EOF

if [ "$FAILED" = 1 ]; then
  echo; [ "$DRY_RUN" = 1 ] && echo "preflight: dry run; the tool check failed" || echo "preflight: FAILED"
  exit 1
fi
if [ "$DRY_RUN" = 1 ]; then echo; echo "preflight: dry run, nothing was executed"; exit 0; fi
echo; echo "preflight: all selected steps passed"
