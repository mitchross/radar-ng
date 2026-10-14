#!/usr/bin/env bash
# Pin every third-party base image used by the Rust deploy to its current digest
# (RUNBOOK §2 open item 9). Rewrites `name:tag` to `name:tag@sha256:...` in place, so the tag
# stays readable and the digest is what actually gets pulled. Idempotent: an existing digest
# is replaced with the current one, so rerun it to refresh pins deliberately.
#
#   deploy/rust/pin-images.sh            # resolve and rewrite
#   deploy/rust/pin-images.sh --check    # resolve and print only, change nothing
#
# Needs docker with buildx (uses `docker buildx imagetools inspect`, which reads the registry
# and doesn't pull). Review the diff and commit it.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
CHECK=0
[ "${1:-}" = "--check" ] && CHECK=1

FILES=(
  "$HERE/Dockerfile"
  "$HERE/docker-compose.yml"
  "$HERE/k8s/basemap-bootstrap-job.yaml"
)

# name:tag references to pin. The open-meteo base is already pinned by digest upstream of this
# script; it is listed so a refresh keeps it current too.
IMAGES=(
  "rust:1.92-bookworm"
  "gcr.io/distroless/cc-debian12:latest"
  "busybox:1.36-musl"
  "alpine:3.22"
  "protomaps/go-pmtiles:latest"
  "postgres:16-alpine"
  "temporalio/auto-setup:1.25.2"
  "temporalio/ui:2.31.2"
  "ghcr.io/open-meteo/open-meteo:1.6.0"
)

command -v docker >/dev/null || { echo "docker not found" >&2; exit 1; }
docker buildx version >/dev/null 2>&1 || { echo "docker buildx not found (pacman -S docker-buildx)" >&2; exit 1; }

fail=0
for ref in "${IMAGES[@]}"; do
  if ! digest=$(docker buildx imagetools inspect "$ref" --format '{{json .Manifest.Digest}}' 2>/dev/null | tr -d '"'); then
    echo "FAIL  $ref (could not resolve)" >&2
    fail=1
    continue
  fi
  case "$digest" in sha256:*) ;; *) echo "FAIL  $ref (unexpected digest '$digest')" >&2; fail=1; continue ;; esac
  echo "$ref@$digest"
  [ "$CHECK" = 1 ] && continue
  # Escape regex metacharacters in the ref (dots, slashes are fine with | as delimiter).
  esc=$(printf '%s' "$ref" | sed 's/[.[\*^$]/\\&/g')
  for f in "${FILES[@]}"; do
    [ -f "$f" ] || continue
    # Match the ref when followed by an optional existing digest and then a non-ref character.
    sed -E -i "s|${esc}(@sha256:[0-9a-f]{64})?([[:space:]\"']\|$)|${ref}@${digest}\\2|g" "$f"
  done
done

if [ "$CHECK" = 0 ]; then
  echo
  echo "Rewritten. Review with: git diff -- deploy/rust"
fi
exit "$fail"
