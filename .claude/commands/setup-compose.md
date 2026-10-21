Bring up radar-ng with Docker Compose on this machine and prove it works. Extra context: $ARGUMENTS

The human-readable version is `docs/self-host/compose.md`; follow it, and use this as the checklist.

1. **Check prerequisites.** `docker version` (Engine 24+), `docker compose version` (v2), `curl`, `jq`.
   About 20 GB free disk and 4+ GB RAM. Stop and tell the user if Docker isn't running.
2. **Settings.** In `deploy/`: `cp .env.example .env` if `.env` doesn't exist. Two values must be set:
   - `NWS_USER_AGENT=(radar-ng, <email>)`: **ask the user for their email**; never invent one.
   - `BASEMAP_PMTILES_URL`: open <https://build.protomaps.com/>, take the **newest** `YYYYMMDD.pmtiles`
     (builds are deleted after ~6 days and there's no "latest"), and write the full URL.
3. **Basemap (once):** `docker compose run --rm basemap-bootstrap` then
   `docker compose run --rm basemap-publish`. A 404 means the date is too old: go back to step 2.
4. **Start:** `docker compose build api` then `docker compose up -d --build`. The first build compiles the
   Rust workspace and takes a while; let it finish.
5. **Verify** (poll, don't sleep blindly; give up after ~10 minutes and go to step 6):
   - `curl -s localhost:8080/api/livez` → 200 right away.
   - `curl -s localhost:8080/api/health` → `degraded` (503) at first, then `"status":"ok"` within ~2–5 min.
   - `curl -s localhost:8080/api/manifest.json | jq '.layers | keys'` lists `radar` and grows over time.
   - `curl -sI localhost:8080/basemap/styles/positron.json` → 200.
6. **If something's wrong,** read `docker compose ps` and `docker compose logs --tail 100 worker api`,
   match the symptom in the "If it didn't work" section of `docs/self-host/compose.md`, fix, re-verify.
   For a deeper sweep use the `doctor` skill.
7. **Report** each check with its actual output, and the app URL to use: `http://localhost:8080`
   (iOS simulator), `http://10.0.2.2:8080` (Android emulator), `http://<LAN-IP>:8080` (a phone).

Don't expose port 8080 to the internet as part of this; that's a separate decision for the user.
