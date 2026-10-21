Health-check a radar-ng stack and explain what's wrong. Target or symptom: $ARGUMENTS

Reference: `docs/reference/debug-harness.md`.

1. **Where is it?** Docker Compose on this machine, a Kubernetes namespace, or a remote URL. Ask if unclear.
2. **Run doctor:**
   - Compose: `docker compose -f deploy/docker-compose.yml exec worker radar debug doctor --server http://api:8080`
   - Kubernetes: from a worker pod, `kubectl -n <ns> exec deploy/<worker> -- radar debug doctor --server http://tile-server:8080`
     (read-only; don't edit workloads by hand on a GitOps-managed cluster).
   - Remote only: `radar debug doctor --server <url> --skip-temporal` from a local build
     (`cargo build --release -p radar`).
   Add `--json` if you need to parse it. Exit code 1 means at least one check failed.
3. **Explain** each `⚠`/`✗` in plain words: what it means for the user (e.g. "radar is 15 minutes old")
   and the most likely cause, using the doc's "Reading the output" section.
4. **Quick checks** if doctor can't run: `curl -s <server>/api/health` (`mrms_age_s`), worker logs, and
   whether Temporal schedules exist.
5. **Suggest the fix**, ordered by likelihood. Don't restart, delete or change anything without asking.
