# radar-ng — session wrap-up & next steps (2026-07-02)

Stopping point for the `feature/weather-clear-reconciliation` branch. Everything
below marked DONE is committed on this branch and verified (tsc + jest + lint
green). Everything under NEXT STEPS is intentionally deferred — either it needs
on-device testing or it's an ops change in the gitops repo.

## Verification baseline
- `cd frontend && bunx tsc --noEmit` → clean
- `cd frontend && bun run test` → 10 suites / 54 tests pass
- `cd frontend && bun run lint` → clean

---

## DONE this session

### Ops incident — tile-server prod outage (resolved)
- `tile-server` was crashlooping (exit 126, `caddy: Operation not permitted`).
  Root cause: caddy binary carried `cap_net_bind_service=ep`; under the pod's
  `allowPrivilegeEscalation:false` (NO_NEW_PRIVS) the kernel refuses to exec a
  cap-bearing binary. Fix (`v1.1.8`, commit `144dcb9`) was already built + in the
  gitops repo, but **ArgoCD had a stale repo-server manifest cache** — it reported
  "Synced" to the v1.1.8 commit while still running v1.1.6.
- Resolved by hard-refreshing the ArgoCD app (user did it). Now `v1.1.8`, healthy.
- **Lesson / tell:** ArgoCD shows Synced/Healthy but no ReplicaSet matches the git
  image tag → stale cache. Fix = `argocd app get <app> --hard-refresh`. Do NOT
  `kubectl set image` as a shortcut (selfHeal reverts while cache is stale).

### App bug fixes (this branch)
1. **7-day forecast day labels off by one** — `frontend/src/app/(tabs)/index.tsx`.
   `forecast.daily.time` are date-only strings (`"2026-07-03"`); `new Date(t)`
   parsed them as UTC midnight → rendered previous local day in US timezones →
   `Today, Thu, Fri…` (Thu duplicated). Fixed by parsing as local:
   `` new Date(`${t}T00:00:00`) ``.
2. **Home radar mini-map zoomed out** — `frontend/src/components/home/RadarMiniMap.tsx`.
   `MINI_ZOOM` was `6` (~1000 km/tile, continental). Bumped to `8` (~250 km/tile,
   metro), matching `DEFAULTS.ZOOM`.
3. **TimelineBar recompute on every playback tick** — `frontend/src/components/timeline/TimelineBar.tsx`.
   The three `findClosestIdx` O(n) scans + segment percentages ran every 420ms
   frame even though they only depend on `frames`/`nowSec`/window. Wrapped in
   `useMemo` (hoisted above the early return for rules-of-hooks).

---

## NEXT STEPS (deferred — prioritized)

### P0 — OPS: open-meteo sync-worker still on the broken base (silent HRRR failure)
- `OpenMeteoSyncWorkflow` (HRRR sync) is FAILING with the **same** libparquet bug
  the `1.5.2` pin was meant to fix: `libparquet-glib.so.2400: cannot open shared
  object file` (rc=127). The pin fixed the *serve* pod (`open-meteo:1.5.2` ✅) but
  the *sync worker* runs `radar-ng-open-meteo-worker:v1.1.2` (pre-fix base).
- The fixed image **`v1.1.3`** IS in the registry (revision `144dcb9`, base 1.5.2).
- **Action (gitops repo `talos-argocd-proxmox`):** bump
  `my-apps/development/radar-ng/deployment-open-meteo.yaml:67`
  `radar-ng-open-meteo-worker:v1.1.2 → v1.1.3`, push, let ArgoCD roll.
  If ArgoCD shows Synced but pod stays v1.1.2 → hard-refresh (see cache tell above).
- Cleanup: `open-meteo-worker-deployment.yaml` is an ORPHAN (also pins v1.1.2, not
  in the kustomization `resources:` list) — delete it to avoid confusion.

### P1 — OPS: temporal-worker-controller ManagerIdentity mismatch
- `workerdeployment/radar-ng-worker` can't promote build `v1.1.11-c857` to the
  ramping version: `ManagerIdentity ... does not match user identity` (two
  temporal-worker-controller instances b05bfd99 vs a4cbb13c fighting). That's why
  both v1.1.8 and v1.1.11 worker pods are running. Investigate which controller
  instance is stale / reset the ManagerIdentity.
- (Temporal core pods show 13-14 restarts but all from ~04:28 UTC — the overnight
  incident; stable since. Not new.)

### P2 — APP: radar map jank (the real fix — needs on-device testing, esp. iOS)
Diagnosed but NOT applied because both viable approaches have documented failure
modes and this is the hot render path (map lib = MapLibre `@maplibre/maplibre-react-native`).

- **Top cause:** `RadarOverlay.tsx` gives `<RasterSource>` a `key` embedding
  `frame.path`, so React remounts the source every frame → native
  `removeSource/addSource` each 420ms, no preload, `raster-fade-duration:0` (no
  crossfade). NOTE the file comment: a prior 7-frame preload caused an **iOS
  NSRangeException** (`insertReactSubview`) from Fragment-wrapped multi-source
  children. So the naive fix crashed iOS before.
  - Candidate A (lower risk): keep a stable source `key`, update only `tiles` prop.
    RISK: MapLibre RasterSource may not reliably reload tiles on in-place `tiles`
    change (likely WHY the key trick exists) → could freeze the overlay. Must
    verify on device.
  - Candidate B (better, more work): mount a small window of **sibling** (NOT
    Fragment-wrapped) `RasterSource`s with stable keys, animate playback via
    per-layer `raster-opacity` toggle so tiles stay warm + crossfade. This is the
    iOS-safe shape per the file's own comment.
- **Second cause:** `TimelineBar.tsx:72-79` playback is `setInterval` → `setState`
  (`setCurrentFrameIndex`) every 420ms → re-render storm across all frame
  subscribers + drives the remount above. Decouple: once a frame window is mounted,
  advance via a shared value / opacity on the UI thread; only sync React state on
  pause/scrub.
- **Wind overlay** `WindParticlesOverlay.tsx`: (a) `useWindField` query key includes
  per-frame `timestamp` → network refetch + full 1200-particle re-seed every frame;
  (b) four separate 1200-iteration `useDerivedValue` worklets (~4800 iters/UI frame).
  Fix: bucket particles in ONE pass; gate field refetch to a coarse cadence / use
  nearest cached field; advect instead of re-seeding.
- **Map memoization (low value):** `WeatherMap` isn't `React.memo`; `radar.tsx`
  passes inline `onLongPress`/`onCameraChanged`. Wrapping helps only on opacity/layer
  toggles (not per-frame). Note `children` identity changes each render, so memo
  needs stable children to actually help. Skipped as low ROI.

### P3 — APP: home mini-map true centering
- Even at z=8 the location can sit anywhere inside its single tile while the pin is
  painted at 50% (`RadarMiniMap.tsx` uses `Math.floor` tile + centered pin). A single
  static tile can't frame an arbitrary point. Proper fix: fetch a 2×2 or 3×3 neighbor
  tile grid and translate by the fractional offset. Deferred (the z=6→8 bump already
  removes the "zoomed way out" complaint).

---

## How to resume
- Branch: `feature/weather-clear-reconciliation` (this branch). The worktree at
  `~/.config/superpowers/worktrees/radar-ng/weather-clear-reconciliation` was removed
  at stop; recreate with `git worktree add <path> feature/weather-clear-reconciliation`
  or just `git checkout feature/weather-clear-reconciliation` in the main repo.
- Nothing has been pushed. Review the commits, then push/merge when ready.
- The emulator was running this branch's build; rebuild after checkout to see the
  three fixes.
