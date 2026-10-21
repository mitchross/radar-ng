# radar-ng docs

Start with **[Getting started](start/getting-started.md)** if you're new: it helps you pick a path and
links to the guide for it.

## I want to…

| …do this | Read |
|---|---|
| Understand what radar-ng is and pick how to run it | [start/getting-started.md](start/getting-started.md) |
| Run the whole stack on one computer (Docker Compose) | [self-host/compose.md](self-host/compose.md) |
| Run it on my own Kubernetes cluster | [self-host/kubernetes.md](self-host/kubernetes.md) · manifests: [../deploy/kubernetes/](../deploy/kubernetes/README.md) |
| Look up an environment variable | [self-host/configuration.md](self-host/configuration.md) |
| Make it faster, cheaper or fresher | [self-host/tuning.md](self-host/tuning.md) |
| Run the phone app and point it at my server | [app/running-the-app.md](app/running-the-app.md) |
| Set up CarPlay or the Apple Watch app | [app/carplay-watch-setup.md](app/carplay-watch-setup.md) |
| Build an Android release | [app/android-release.md](app/android-release.md) |
| Know what the app sends, and to whom | [app/privacy.md](app/privacy.md) |
| Contribute code | [../CONTRIBUTING.md](../CONTRIBUTING.md) |
| Fork it with my own app name, server and images | [../FORKING.md](../FORKING.md) |
| Work on the Rust backend | [develop/rust-backend-handoff.md](develop/rust-backend-handoff.md) · [../rust/README.md](../rust/README.md) · [develop/specs/](develop/specs/) |
| Cut a release | [develop/releasing.md](develop/releasing.md) · [develop/temporal-safe-deployments.md](develop/temporal-safe-deployments.md) |
| Upgrade Expo / React Native | [develop/expo-rn-upgrade-playbook.md](develop/expo-rn-upgrade-playbook.md) |
| See how the pieces fit together | [reference/architecture.md](reference/architecture.md) |
| Debug a running stack | [reference/debug-harness.md](reference/debug-harness.md) |
| Understand packed data tiles | [reference/packed-tiles.md](reference/packed-tiles.md) |
| Load-test it | [reference/capacity-acceptance.md](reference/capacity-acceptance.md) · [../load/README.md](../load/README.md) |
| Check data-source licenses and terms | [reference/third-party-terms.md](reference/third-party-terms.md) |
| Read the product principles | [reference/north-star.md](reference/north-star.md) |

## Layout

```
docs/
  start/       new here? begin here
  self-host/   run the backend: Compose, Kubernetes, configuration, tuning
  app/         the Expo app: running, device builds, CarPlay/Watch, privacy
  develop/     working on the code: backend handoff, API/worker specs, releasing
  reference/   architecture, debugging, packed tiles, data terms
  archive/     dated QA, perf and review notes, and the Python → Rust migration history
  diagrams/    SVGs used across the docs
  screenshots/ app screenshots used by the README
```

`archive/` is kept for history and is not maintained. If something there conflicts with a page
above, the page above wins.
