# Agent Instructions

For any coding agent (Claude Code, Codex, Cursor, Pi, Hermes…) working in this repo. Humans: the same
rules are in [CONTRIBUTING.md](CONTRIBUTING.md).

## The repo in one paragraph

`rust/` is the backend: one binary, `radar`, runs the API/tile server (`radar api`) and the Temporal
workers (`radar worker --role …`). `frontend/` is the Expo phone app, `web/` the web radar,
`deploy/` the Dockerfile, Docker Compose stack and Kubernetes manifests (`deploy/kubernetes/`). Docs
are indexed in [docs/README.md](docs/README.md); words are in [docs/start/glossary.md](docs/start/glossary.md).

## Skills: load the one that matches the task

| Task | Skill (`.claude/commands/<name>.md`, `.agents/skills/<name>/SKILL.md`) |
|---|---|
| **Any change under `rust/`**, the Dockerfile or backend CI | **`rust-dev`**: read it first, every time |
| Run and verify the local stack with Docker Compose | `setup-compose` |
| Deploy to a Kubernetes cluster (kind, k3d, k3s or real) | `setup-k8s` |
| Make a fork your own (app IDs, server URL, images) | `fork-setup` |
| Add a new map layer | `add-layer` |
| Check a running stack's health | `doctor` |

## Git

You may `git add`/`commit`/`push` without asking **only** on a new feature branch, to open a PR.
Anything else needs an explicit instruction. Never push to the default branch (`master`).

## Verify before you say "done"

| You changed | Run (all must pass) |
|---|---|
| `rust/` | `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (in `rust/`; see `rust-dev`) |
| `frontend/` | `bun run lint`, `bunx tsc --noEmit`, `bun run test` (in `frontend/`) |
| `web/` | `bun run test`, `bun run build` (in `web/`) |
| `deploy/kubernetes/` | `deploy/kubernetes/validate.sh` |
| app identity / URLs | `node scripts/sync-native-identity.js --check` (in `frontend/`; see [FORKING.md](FORKING.md)) |
| docs or `llms.txt` | every relative link resolves (CI checks `llms.txt`) |

Say plainly what you could not run (no Xcode, no Docker, no cluster) instead of claiming it works.

## Web + app parity

Every user-facing feature ships to **both** the web radar (`web/`) and the app (`frontend/`) in the
same change, never one without the other. When planning, list the web piece, the app piece, and what
is server-side (automatic for both). App changes reach a phone only after a native rebuild
(`bun run ios:ipa` / an Android build); say so when handing off.

## Backend is Rust

The Python backend was removed on 2026-10-14. History and migration gotchas are in
[docs/develop/rust-backend-handoff.md](docs/develop/rust-backend-handoff.md); how to work in `rust/`
today is the `rust-dev` skill.

## Upstream maintainer only

These apply to the upstream repo (`mitchross/radar-ng`) and its owner's tools. In a fork, skip them.

- **Releases:** GHCR `vX.Y.Z` → a tag@digest PR in `mitchross/talos-argocd-proxmox` → Argo CD.
  GitOps only, never `kubectl apply`. Forward-only: fix a bad release with a new release, never roll
  back. Details: [docs/develop/releasing.md](docs/develop/releasing.md).
- **Mink** (only if `command -v mink` succeeds): keep Mink updated during substantive work. Durable
  decisions, verified root causes, runbooks and gotchas go in with
  `mink note --project radar-ng --category resources` (or `--category projects` for active decisions
  and follow-ups). Don't capture routine edits, raw output or unverified guesses. Mention saved note
  paths in your final response.
- **Personal additions** belong in git-ignored `CLAUDE.local.md` / `AGENTS.local.md` /
  `.claude/settings.local.json`, not in the committed files.
