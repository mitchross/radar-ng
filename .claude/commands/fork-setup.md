Make this fork of radar-ng the user's own. Extra context: $ARGUMENTS

Follow `FORKING.md`; this is the checklist for doing it.

1. Read the "Before you fork" notes in `FORKING.md` (from `docs/reference/third-party-terms.md`) and
   **tell the user** about the satellite imagery licensing risk before they plan a store release.
2. **Ask the user** for: iOS bundle ID, Apple Team ID, Android package name, their server URL, and
   (optional) their basemap raster URL. Don't guess identifiers.
3. Write them to `frontend/.env` (git-ignored): `RADAR_BUNDLE_ID`, `RADAR_APPLE_TEAM_ID`,
   `RADAR_ANDROID_PACKAGE`, `EXPO_PUBLIC_RADAR_DEFAULT_URL`, optionally `RADAR_APP_GROUP` and
   `RADAR_BASEMAP_RASTER_URL`. Values are read by `frontend/identity.js`.
4. In `frontend/`: `node scripts/sync-native-identity.js`, then `--check` (must print nothing). Commit
   the Swift changes it made.
5. Check the result: `npx expo config --type public --json` shows their bundle ID, team and package.
6. `frontend/.env.production` points release builds at upstream's map styles: repoint or delete those
   lines (ask which).
7. Server side: `NWS_USER_AGENT` with their email in `deploy/.env` or `deploy/kubernetes/base/settings.env`.
8. Images: nothing to edit; workflows publish to `ghcr.io/<their-github-user>/radar-ng-*`. Remind them to
   point deployments at their own images when ready.
9. Run `bun run test` in `frontend/` (includes the identity check). Tell the user a **native rebuild**
   is required and that you couldn't sign or build iOS without their Xcode/Apple account.
