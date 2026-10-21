Add a new map layer to radar-ng. Layer: $ARGUMENTS

**Parity rule:** a user-facing layer ships to **both** `web/` and `frontend/` in the same PR. Before
coding, write a short plan with three parts: server (Rust), web, app.

## Find the pattern, then copy it

Pick the existing layer closest to the new one (gridded model field → `radar-hrrr` or the air-quality
layers; observed radar → `radar`; points → lightning). Then find every place it appears:

```bash
git grep -n '<existing-layer-id>' -- rust web/src frontend/src frontend/targets
```

Expect touch points like these (confirm with the grep; don't trust this list blindly):
- **Rust:** ingest/decoding (`radar-grib`, `radar-aux`, `radar-worker`), rendering and palettes
  (`radar-tiler`, `radar-api/src/palettes.rs`), the manifest that advertises frames, workflow/schedule
  wiring in `radar-types` (a **new** workflow or schedule is fine; changing existing ones is a replay
  risk, see `rust-dev`).
- **Web:** `web/src/main.ts`, `frames.ts`, `overlays.ts`, `layerLegends.ts`.
- **App:** `frontend/src/components/map/radarLayers.ts`, `LayerLegendCard.tsx`, `src/types/weather.ts`,
  `src/hooks/useManifest.ts`, the layer picker in `app/(tabs)/radar.tsx` and `settings.tsx`,
  `src/lib/inspector.ts` for point readouts.

## Rules

- Load the `rust-dev` skill before touching `rust/` and run its gate.
- The apps discover layers from `/api/manifest.json`; a layer with no frames simply isn't offered, so
  the apps must handle its absence.
- Data must come from a free, keyless public source; record its terms in
  `docs/reference/third-party-terms.md`.
- Add tests on each side (Rust unit/fixture tests, `web` vitest, `frontend` jest) and document new env
  vars in `docs/self-host/configuration.md`.
- Hand-off: list the web piece, the app piece, the server piece, and note the app needs a native rebuild.
