#!/usr/bin/env sh
# One command to run radar-ng with Docker Compose:
#   settings check → basemap (once) → start → wait for the first radar frame → print the app URL.
#
#   ./up.sh            # prebuilt images from GHCR (linux/amd64)
#   ./up.sh --build    # build the images from this checkout (arm64 hosts, or after changing code)
#
# Safe to re-run: the basemap step skips itself when it's already done, and `up -d` only changes what changed.
set -eu
cd "$(dirname "$0")"

say() { printf '\n==> %s\n' "$*"; }
die() { printf '\nERROR: %s\n' "$*" >&2; exit 1; }

files="-f docker-compose.yml"
build=""
case "${1:-}" in
  --build) files="$files -f docker-compose.build.yml"; build="--build" ;;
  "") ;;
  -h | --help) sed -n '2,8p' "$0"; exit 0 ;;
  *) die "unknown option: $1 (use --build or nothing)" ;;
esac

# --- 1. Tools ---------------------------------------------------------------------------------------
command -v docker >/dev/null 2>&1 || die "Docker isn't installed. Get Docker Engine 24+ (Linux) or Docker Desktop/OrbStack (Mac)."
docker info >/dev/null 2>&1 || die "Docker is installed but not running (or you need sudo / the docker group)."
docker compose version >/dev/null 2>&1 || die "The 'docker compose' plugin (Compose v2) is missing."
command -v curl >/dev/null 2>&1 || die "curl is needed for the health check."

if [ -z "$build" ]; then
  case "$(uname -m)" in
    arm64 | aarch64)
      die "This is an arm64 machine, and the prebuilt images are amd64 only. Run: ./up.sh --build" ;;
  esac
fi

# --- 2. Settings ------------------------------------------------------------------------------------
if [ ! -f .env ]; then
  cp .env.example .env
  say "Created deploy/.env from .env.example"
fi
ua=$(sed -n 's/^NWS_USER_AGENT=//p' .env | tail -n1)
case "$ua" in
  "" | *you@example.com*)
    die "Set your contact email in deploy/.env first, e.g.
  NWS_USER_AGENT=(radar-ng, me@example.org)
The National Weather Service asks every app for one. Then run ./up.sh again." ;;
esac
port=$(sed -n 's/^TILE_SERVER_PORT=//p' .env | tail -n1)
port=${port:-8080}

# --- 3. Basemap (one-time; skips itself when already done) -------------------------------------------
say "Basemap: downloading the newest US map once (~9 GB; skipped if already there)"
# shellcheck disable=SC2086
docker compose $files run --rm basemap-bootstrap || die "The basemap download failed. See 'Blank / gray basemap' in docs/self-host/troubleshooting.md."

# --- 4. Start -------------------------------------------------------------------------------------
if [ -n "$build" ]; then
  say "Building the images from this checkout (the first build takes a while)"
fi
say "Starting the stack"
# shellcheck disable=SC2086
docker compose $files up -d $build || die "docker compose up failed. See docs/self-host/troubleshooting.md (port in use, image pull, Temporal)."

# --- 5. Wait for the first radar frame -------------------------------------------------------------
say "Waiting for the API (http://localhost:${port})"
i=0
until curl -fsS "http://localhost:${port}/api/livez" >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -le 60 ] || die "The API didn't come up in 5 minutes. Check: docker compose logs api"
  sleep 5
done

say "API is up. Waiting for the first radar frame (usually ~2 minutes, up to 10)"
i=0
until curl -fsS "http://localhost:${port}/api/health" >/dev/null 2>&1; do
  i=$((i + 1))
  if [ "$i" -gt 120 ]; then
    printf '\nStill no radar after 10 minutes. The stack is running; check: docker compose logs worker\n'
    printf 'and docs/self-host/troubleshooting.md ("/api/health stays degraded").\n'
    exit 1
  fi
  sleep 5
done

# `hostname -I` is GNU-only (Arch's inetutils and macOS lack it); fall back to the default route.
ip=$(hostname -I 2>/dev/null | awk '{print $1}' || true)
[ -n "$ip" ] || ip=$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{for (i = 1; i < NF; i++) if ($i == "src") print $(i + 1)}' || true)
[ -n "$ip" ] || ip=$(ipconfig getifaddr en0 2>/dev/null || true)
ip=${ip:-YOUR-COMPUTER-IP}
cat <<EOF

radar-ng is running. Point an app at it (Settings → Advanced Mode → Stack URL):
  iOS simulator on this machine:  http://localhost:${port}
  Android emulator:               http://10.0.2.2:${port}
  A phone on your Wi-Fi:          http://${ip}:${port}
  Web radar (dev):                cd ../web && RADAR_API=http://localhost:${port} bun run dev

More layers (forecast, HRRR, nowcast) appear over the next 15-30 minutes.
Stop:  docker compose down      (add -v to delete all data)
EOF
