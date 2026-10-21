#!/usr/bin/env bash
# Render every example and every component (on top of base) and, when kubeconform is on PATH,
# validate the output against the Kubernetes schemas plus the community CRD catalog.
#
#   deploy/kubernetes/validate.sh            # from anywhere; needs kubectl (or kustomize)
#
# CRDs without a published schema (the Temporal Worker Controller's) are skipped, not failed.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if command -v kustomize >/dev/null 2>&1; then
  build() { kustomize build "$1"; }
else
  build() { kubectl kustomize "$1"; }
fi

validate() {
  if command -v kubeconform >/dev/null 2>&1; then
    kubeconform -strict -summary -ignore-missing-schemas \
      -schema-location default \
      -schema-location 'https://raw.githubusercontent.com/datreeio/CRDs-catalog/main/{{.Group}}/{{.ResourceKind}}_{{.ResourceAPIVersion}}.json' \
      "$1"
  fi
}

targets=("$here/base")
for d in "$here"/examples/*/; do targets+=("${d%/}"); done

# Each component on its own, layered on base, so a component no example uses still gets built.
cp -r "$here/base" "$here/components" "$tmp/"
for c in "$here"/components/*/; do
  name="$(basename "$c")"
  mkdir -p "$tmp/$name"
  cat >"$tmp/$name/kustomization.yaml" <<EOF
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization
resources: [../base]
components: [../components/$name]
EOF
  targets+=("$tmp/$name")
done

fail=0
for t in "${targets[@]}"; do
  label="${t#"$here"/}"
  label="${label#"$tmp"/}"
  out="$tmp/$(echo "$label" | tr '/' '_').rendered.yaml"
  if ! build "$t" >"$out"; then
    echo "FAIL build: $label" >&2
    fail=1
    continue
  fi
  echo "ok build: $label ($(grep -c '^kind:' "$out") objects)"
  validate "$out" || { echo "FAIL validate: $label" >&2; fail=1; }
done
exit "$fail"
