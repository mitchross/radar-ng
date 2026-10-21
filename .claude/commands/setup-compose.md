Bring up radar-ng with Docker Compose on this machine and prove it works. Extra context: $ARGUMENTS

The human-readable version is `docs/self-host/compose.md`; follow it, and use this as the checklist.

1. **Check prerequisites.** `docker version` (Engine 24+), `docker compose version` (v2), `curl`, `jq`.
   About 30 GB free disk and 4+ GB RAM. Stop and tell the user if Docker isn't running.
2. **Settings.** In `deploy/`: `cp .env.example .env` if `.env` doesn't exist. Set
   `NWS_USER_AGENT=(radar-ng, <email>)`: **ask the user for their email**; never invent one. Leave
   `BASEMAP_PMTILES_URL` empty (the bootstrap finds the newest build).
3. **Run `./up.sh`** (on arm64, `uname -m` = `arm64`/`aarch64`: `./up.sh --build`; the GHCR images are
   amd64 only). It downloads the basemap once, starts the stack and waits for the first radar frame.
   By hand instead: `docker compose run --rm basemap-bootstrap`, then `docker compose up -d` (or
   `docker compose -f docker-compose.yml -f docker-compose.build.yml up -d --build` to build locally).
4. A local build compiles the Rust workspace and takes a while; let it finish.
5. **Verify** (poll, don't sleep blindly; give up after ~10 minutes and go to step 6):
   - `curl -s localhost:8080/api/livez` → 200 right away.
   - `curl -s localhost:8080/api/health` → `degraded` (503) at first, then `"status":"ok"` within ~2–5 min.
   - `curl -s localhost:8080/api/manifest.json | jq '.layers | keys'` lists `radar` and grows over time.
   - `curl -sI localhost:8080/basemap/styles/positron.json` → 200.
6. **If something's wrong,** read `docker compose ps` and `docker compose logs --tail 100 worker api`,
   match the symptom in `docs/self-host/troubleshooting.md`, fix, re-verify.
   For a deeper sweep use the `doctor` skill.
7. **Report** each check with its actual output, and the app URL to use: `http://localhost:8080`
   (iOS simulator), `http://10.0.2.2:8080` (Android emulator), `http://<LAN-IP>:8080` (a phone).

Don't expose port 8080 to the internet as part of this; that's a separate decision for the user.
