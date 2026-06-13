#!/usr/bin/env bash
# Build and push all radar-ng container images to registry.vanillax.me.
# Registry is anonymous-push (no docker login needed).
#
# Usage:
#   ./scripts/build-push.sh            # build + push everything
#   ./scripts/build-push.sh tile-server # just one
#
# Image naming:
#   registry.vanillax.me/radar-ng-{service}:latest
#   registry.vanillax.me/radar-ng-{service}:git-<short-sha>
#   registry.vanillax.me/radar-ng-{service}:$VERSION  (optional; e.g. v1.0.1)

set -euo pipefail

REGISTRY="${REGISTRY:-registry.vanillax.me}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SHA="$(cd "$REPO_ROOT" && git rev-parse --short HEAD 2>/dev/null || echo dirty)"

# Map: service => "<dockerfile-rel-path>;<context-rel-path>"
# The per-service ingest/nowcast images (and the radar-ng-base they FROM'd)
# were deleted with the legacy daemons — every activity now ships in the
# temporal-worker image. Only the four deployed images remain.
declare -A SERVICES=(
  [tile-server]="backend/api/Dockerfile;backend"
  [basemap]="backend/basemap/Dockerfile;."
  [temporal-worker]="temporal/Dockerfile;."
  [open-meteo-worker]="temporal/open_meteo_worker.Dockerfile;."
)

build_push() {
  local name="$1"
  local spec="${SERVICES[$name]}"
  if [[ -z "$spec" ]]; then
    echo "Unknown service: $name"
    exit 1
  fi
  local dockerfile="${spec%%;*}"
  local context="${spec##*;}"
  local latest_tag="${REGISTRY}/radar-ng-${name}:latest"
  local sha_tag="${REGISTRY}/radar-ng-${name}:git-${SHA}"
  local extra_args=()
  local extra_tags=()
  if [[ -n "${VERSION:-}" ]]; then
    local ver_tag="${REGISTRY}/radar-ng-${name}:${VERSION}"
    extra_args+=(-t "$ver_tag")
    extra_tags+=("$ver_tag")
  fi

  echo ""
  echo "──────────────────────────────────────────"
  echo "  $name"
  echo "  dockerfile: $dockerfile"
  echo "  context:    $context"
  echo "  tags:       $latest_tag, $sha_tag${extra_tags:+, ${extra_tags[*]}}"
  echo "──────────────────────────────────────────"

  docker build \
    -t "$latest_tag" \
    -t "$sha_tag" \
    "${extra_args[@]}" \
    -f "$REPO_ROOT/$dockerfile" \
    "$REPO_ROOT/$context"

  docker push "$latest_tag"
  docker push "$sha_tag"
  for t in "${extra_tags[@]}"; do
    docker push "$t"
  done
  echo "[done] $name"
}

TARGETS=()
if [[ $# -eq 0 ]]; then
  TARGETS=(tile-server basemap temporal-worker open-meteo-worker)
else
  TARGETS=("$@")
fi

for t in "${TARGETS[@]}"; do
  build_push "$t"
done

echo ""
echo "All done. Pushed to $REGISTRY"
