# Contributing to radar-ng

Thanks for your interest! radar-ng is a monorepo with a Python ingest/tile
pipeline and an Expo mobile app. This doc covers how to get set up and the
conventions the project follows.

## Getting set up

See **[GETTING_STARTED.md](GETTING_STARTED.md)** — it covers running the stack
(Docker Compose or Kubernetes) and the mobile app.

- **Backend / pipeline:** Python 3.12. Code lives in `backend/` (activities) and `temporal/` (workflows + workers).
- **Frontend:** Expo (SDK 58 preview) + TypeScript, package manager is **[Bun](https://bun.sh)**.
  ```bash
  cd frontend
  bun install
  bunx tsc --noEmit   # typecheck
  bun run test        # jest
  bun run lint        # expo lint
  bunx expo start
  ```

  Jest runs two projects. `unit` covers pure logic and config plugins with ts-jest in node.
  `render` mounts real components under the jest-expo preset (`__tests__/render`, `*.test.tsx`)
  and asserts on roles, labels and behaviour, not on source text. Native-only modules are
  stubbed in `__tests__/render/setup.tsx`; wrap components in `renderWithProviders`.

## Making changes

1. **Branch** off `master`.
2. Keep changes focused; match the style and comment density of the surrounding code.
3. **Verify before you push:**
   - Frontend: `bunx tsc --noEmit`, `bun run test`, and `bun run lint` must all pass.
   - Backend: run the relevant tests under `backend/`.
4. Open a PR against `master` with a clear description of the what and the why.

## Self-hosted only

Clients (phone, Watch, widget) talk **only** to the operator's own servers at runtime. That rules out
API keys and third-party map, imagery, geocoding or tile services, including keyless platform ones
like MapKit snapshots and CLGeocoder. Public data is fine when the *server* fetches it: the tile
server proxies NWS alerts, Photon geocoding and USGS imagery. `frontend/__tests__/selfHosted.test.ts`
and the bundled-style check in `backend/api/api/test_routes_places.py` enforce this. The CarPlay
target's Apple routing is the one documented exception while its entitlement is pending.

## Commit / PR conventions

- Conventional-commit-style prefixes are appreciated (`fix:`, `feat:`, `docs:`, `perf:`, `refactor:`).
- Explain *why* in the body when the change isn't obvious — this repo's comments and commit messages lean toward capturing rationale, not just what changed.

## Images & releases

First-party images publish to GHCR (`ghcr.io/mitchross/radar-ng-*`) via
`.github/workflows/`. Release/versioning notes are in
[docs/releasing.md](docs/releasing.md).

## Reporting bugs / requesting features

Use the issue templates. For anything security-sensitive, please **do not** open
a public issue — see [SECURITY.md](SECURITY.md) if present, or email the
maintainer.

## License

By contributing, you agree that your contributions are licensed under the
project's [MIT License](LICENSE).
