# Contributing to radar-ng

Thanks for your interest! radar-ng is a monorepo with a Rust ingest/tile pipeline and API, an Expo
mobile app, and a web radar. This doc covers how to get set up and the conventions the project
follows.

## Getting set up

See **[docs/start/getting-started.md](docs/start/getting-started.md)** — it covers running the stack
(Docker Compose or Kubernetes) and the mobile app.

- **Backend / pipeline:** Rust, one Cargo workspace in `rust/` (toolchain pinned in `rust/rust-toolchain.toml`; `protoc` required). See [rust/README.md](rust/README.md).
  ```bash
  cd rust
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```
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
   - Web (`web/`): `bun run build` (typecheck + Vite build) and `bun run test`.
   - Backend: `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test` for the crates you touched. CI (`ci-rust.yml`) also runs the Temporal integration tests, the live-NOAA check and the image builds.
   - User-facing features ship to both the app (`frontend/`) and the web radar (`web/`) in the same change.
4. Open a PR against `master` with a clear description of the what and the why.

Using a coding agent? [AGENTS.md](AGENTS.md) lists repo skills (`rust-dev`, `setup-compose`,
`setup-k8s`, `fork-setup`, `add-layer`, `doctor`) in `.claude/commands/` and `.agents/skills/`.

## Self-hosted only

Clients (phone, Watch, widget) talk **only** to the operator's own servers at runtime. That rules out
API keys and third-party map, imagery, geocoding or tile services, including keyless platform ones
like MapKit snapshots and CLGeocoder. Public data is fine when the *server* fetches it: the tile
server proxies NWS alerts, Photon geocoding and USGS imagery. `frontend/__tests__/selfHosted.test.ts`
enforces this for the app. The CarPlay target's Apple routing is the one documented exception while
its entitlement is pending.

## Commit / PR conventions

- Conventional-commit-style prefixes are appreciated (`fix:`, `feat:`, `docs:`, `perf:`, `refactor:`).
- Explain *why* in the body when the change isn't obvious — this repo's comments and commit messages lean toward capturing rationale, not just what changed.

## Images & releases

First-party images publish to GHCR (`ghcr.io/mitchross/radar-ng-*`) via
`.github/workflows/`. Release/versioning notes are in
[docs/develop/releasing.md](docs/develop/releasing.md).

## Reporting bugs / requesting features

Use the issue templates. For anything security-sensitive, please **do not** open
a public issue. Contact the maintainer privately instead (see the
[GitHub profile](https://github.com/mitchross)).

## License

By contributing, you agree that your contributions are licensed under the
project's [MIT License](LICENSE).
