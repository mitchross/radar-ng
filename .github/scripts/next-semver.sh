#!/usr/bin/env bash
# Pick the next release tag for a set of public GHCR images and print it on stdout.
#
#   next-semver.sh OWNER FLOOR IMAGE...      # EXPLICIT=vX.Y.Z overrides the bump
#
# Lists every tag of every image (GHCR caps a page at 100 unless ?n= is given, and
# may still add a rel="next" link), takes the highest vX.Y.Z across all of them and
# bumps the patch, so all images in a release share one new tag. The tag is never
# below FLOOR. Any HTTP or auth error fails the script, and so does a chosen tag
# that already exists on any of the images. Read-only: it only pulls tag lists.
set -euo pipefail

[ "$#" -ge 3 ] || { echo "usage: $0 OWNER FLOOR IMAGE..." >&2; exit 2; }
owner=$1 floor=$2
shift 2
semver='^v[0-9]+\.[0-9]+\.[0-9]+$'
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

max=""
for image in "$@"; do
  token=$(curl -fsS "https://ghcr.io/token?scope=repository:${owner}/${image}:pull" | jq -er .token)
  url="https://ghcr.io/v2/${owner}/${image}/tags/list?n=10000"
  : > "$tmp/$image"
  while [ -n "$url" ]; do
    curl -fsS -D "$tmp/headers" -H "Authorization: Bearer $token" "$url" | jq -er '.tags // [] | .[]' >> "$tmp/$image"
    next_path=$(sed -n 's/^[Ll]ink: *<\([^>]*\)>.*rel="next".*/\1/p' "$tmp/headers" | tr -d '\r')
    url=""
    if [ -n "$next_path" ]; then url="https://ghcr.io$next_path"; fi
  done
  top=$(grep -E "$semver" "$tmp/$image" | sort -V | tail -1 || true)
  echo "$image: $(wc -l < "$tmp/$image") tags, max ${top:-none}" >&2
  max=$(printf '%s\n%s\n' "$max" "$top" | grep -E "$semver" | sort -V | tail -1 || true)
done

if [ -n "${EXPLICIT:-}" ]; then
  next=$EXPLICIT
  [[ $next =~ $semver ]] || { echo "::error::version '$next' is not vX.Y.Z" >&2; exit 1; }
elif [ -z "$max" ]; then
  next=$floor
else
  IFS=. read -r major minor patch <<< "${max#v}"
  next="v$major.$minor.$((patch + 1))"
fi
if [ "$(printf '%s\n%s\n' "$next" "$floor" | sort -V | head -1)" != "$floor" ]; then
  [ -z "${EXPLICIT:-}" ] || { echo "::error::version $next is below the floor $floor" >&2; exit 1; }
  next=$floor
fi

for image in "$@"; do
  if grep -qxF "$next" "$tmp/$image"; then
    echo "::error::$next already exists on ghcr.io/$owner/$image" >&2
    exit 1
  fi
done
echo "next $next (highest existing: ${max:-none})" >&2
echo "$next"
