# radar-alerts

Plain async implementations of the alert, lightning and storm-watch activities for the radar-ng Rust port, plus the
APNs and FCM push senders. The crate has no Temporal types. The worker crate wraps each function as an activity with
the same name, retry policy and timeouts.

Sources of truth: `specs/spec-ingest-aux.md` (sections 6, 10), `specs/spec-temporal.md` (sections 4.10 to 4.12 and 8),
`specs/spec-shared.md` (sections 8 and 9), and `docs/PLAN.md` (Python defects table).

## Coverage

| Area | Module | Python origin |
|---|---|---|
| Blitzortung websocket consumer, LZW decode, strike buffer, `lightning.json` writer | `lightning` | `backend/ingest_lightning/activities.py` |
| NWS active alerts, seen-set state, watch matching | `alerts` | `backend/api/api/storm_watch_activities.py` |
| Push-token store, frame comparison, change detection, push fan-out | `storm_watch` | `backend/api/api/storm_watch_activities.py` |
| `send_push_notification`, APNs (HTTP/2, ES256 JWT), FCM (HTTP v1, service-account OAuth) | `push` | `temporal/shared/push.py` |

## Public API

- `lightning::lightning_consume_stream(config, duration, cancel, heartbeat) -> Result<LightningRunResult, LightningError>`.
  `LightningConfig::from_env()` reads `STATE_DIR`, `LIGHTNING_RETENTION_MIN`, `LIGHTNING_MAX_STRIKES`,
  `LIGHTNING_FLUSH_S`, `LIGHTNING_WS_ENDPOINTS` (replaces the primary list only), and the `LIGHTNING_LAT_MIN`,
  `LIGHTNING_LAT_MAX`, `LIGHTNING_LON_MIN`, `LIGHTNING_LON_MAX` box. Also exported: `decode_payload`, `decode_frame`,
  `extract_strike`, `EndpointPicker`, `StrikeBuffer`, `load_existing_strikes`, `render_geojson`, `write_geojson`.
- `alerts::fetch_nws_active_alerts(&AlertsConfig)`: fetch and diff against the seen set. It does not commit the seen
  set. It writes `alerts_active_snapshot.json`.
- `alerts::mark_alerts_seen(&AlertsConfig, &[String])`: commit handled IDs to `alerts_seen.json`. Idempotent.
- `alerts::signal_matching_storm_watches(&dyn WatchSignaler, &SignalWatchesInput)`. The `WatchSignaler` trait has
  `list_running_watches`, `watch_center` and `signal_alert_match`. The worker implements it over the Temporal client.
- `storm_watch::{persist_push_token, delete_push_token, compare_radar_frames, detect_storm_change, fan_out_push_to_user}`.
- `push::{PushConfig, PushSender, PushError, send_push_notification, push_disabled_from}`. `PushConfig::from_env()`
  reads `PUSH_DISABLED`, `APNS_HOST`, `APNS_TOPIC`, `APNS_KEY_ID`, `APNS_TEAM_ID`, `APNS_KEY_PATH`, `FCM_PROJECT_ID`,
  `FCM_CREDENTIALS_PATH`.

`PushError` has two variants, `Retryable` and `NonRetryable`. The variant decides the worker's retry behaviour, which
matches the Python `ApplicationError(non_retryable=True)` split.

## Tests

`cargo test -p radar-alerts`: **84 passed, 0 failed**. Zero compiler warnings.

| Target | Count | What it covers |
|---|---|---|
| unit tests (`src/`) | 46 | LZW spec vectors and caps, frame coercion, endpoint picker rules, strike seeding and pruning, `lightning.json` writer, point-in-polygon, seen-set commit and cap, NWS feature parsing, `PUSH_DISABLED` matrix, collapse-ID truncation, FCM form encoding and body shape |
| `tests/golden.rs` | 9 | Replays of the Python outputs: 806 LZW cases, 5280 point-in-polygon cases, 43 picker scripts, 7 seed cases, the writer bytes, 11 alert-feature cases, 38 seen-state cases, the snapshot bytes. Also hole and outer-ring invariants |
| `tests/lightning_ws.rs` | 5 | Local websocket server: strike persistence, bad-frame skipping, binary and LZW text frames, reconnect after close, exact subscribe frame, heartbeat fields, cancellation, seeding inside the retention window, refused endpoint |
| `tests/nws_alerts.rs` | 7 | axum mock: `User-Agent` and `Accept` headers, feed-order diff with duplicates, snapshot bytes, mark-seen idempotency, 500 status, non-list features. Fake `WatchSignaler`: matches only inside the polygon, per-watch failure swallowed, list failure fails the call, empty geometry skips listing |
| `tests/push_senders.rs` | 12 | axum mock for APNs and FCM: request path, headers and body match the Python golden. APNs JWT signature verified with `testdata/keys/apns-test.pub.pem`. FCM assertion verified with `fcm-test.pub.pem`. Status replays for dead-token deletion and retryability. OAuth token caching. Disabled gate. Fan-out partial and total failure. Token persist and delete |
| `tests/storm_watch.rs` | 5 | Synthetic radar grid (meta plus f32 `.bin`): sampling, same-frame suppression, out-of-grid, fill values. Change-detection threshold table and configured thresholds |

Golden fixtures live in `testdata/`. They were generated from the original Python with
`RADAR_NG_ROOT=<radar-ng checkout> <venv>/bin/python testdata/gen_golden.py`, run from the crate directory.
`testdata/keys/` holds throwaway test keys. They protect nothing.

## Deviations from the Python behaviour

1. **PUSH_DISABLED (PLAN D5, fixed).** Only `1`, `true` and `yes` disable sends, compared after trimming and
   lowercasing. Unset disables, as the Python default `"1"` does. `""`, `"0"` and `"false"` enable sends. Python
   disabled only for the exact string `"1"`.
2. **APNs collapse ID (spec D15, replicated and documented).** The collapse ID is cut to 64 characters, as the Python
   slice does. The APNs limit is in bytes. The IDs are ASCII, so the two agree in practice.
3. **Malformed lightning frames (spec D-4, skip and count).** A frame that parses but is not an object, or has a field
   Python cannot coerce, is skipped. It is logged once per session and counted in the run total. It does not end the
   session. The owner has not signed this off yet.
4. **Decode caps.** Python has no size limit on LZW output. This port returns the raw frame when the decoded output
   exceeds 4 MiB or the dictionary exceeds 8 MiB. Real frames decode to a few hundred bytes.
5. **Non-object `properties` in an NWS feature** is treated as `{}`. Python raised `AttributeError`, which aborted the
   whole fetch.
6. **Lightning cancellation** returns `LightningError::Cancelled` without a final flush, as the Python
   `CancelledError` path does. The heartbeat cadence is `LightningConfig::heartbeat_every`, which is 30 s in production.
   The tests shorten it.
7. **D-8c, NWS User-Agent.** The default `NWS_USER_AGENT` is the Python source default, `(radar-ng, mitch@example.com)`.
   Production must set a real contact. The default is a placeholder, not a decision.
8. **Push fan-out `sent`** counts successful sends and, with push disabled, no-op sends too (spec D6). It is not a
   delivery count. A partial failure returns `Ok`. The call errors only when every send fails.
9. **Dead tokens** are deleted before the non-retryable error is returned. APNs: status 410, or a body containing
   `BadDeviceToken` or `Unregistered`. FCM: status 404, or a body containing `NOT_REGISTERED` or
   `INVALID_ARGUMENT`.
10. **Credentials.** The APNs provider JWT is minted per send, as Python does. The FCM access token is cached for 3000 s
    and reused while more than 60 s remain. The FCM assertion `aud` is fixed to the Google token endpoint, as in Python.
    Redirects are not followed.
11. **Non-200 2xx from APNs or FCM** is classified as a retryable 5xx with a misleading message. Both services return
    only 200 on success, so this path is not expected to occur.

## Deferred and not verified

- **Live services.** This environment cannot reach NOAA, Apple, Google or the Blitzortung hosts. Every external path is
  tested against a local mock or fixture, not against the real service.
- **APNs over HTTP/2 and TLS.** The mock uses `http://`, which runs HTTP/1.1. The h2 negotiation over TLS that APNs needs
  is untested.
- **Temporal wiring.** `WatchSignaler` is implemented in the worker crate. The query and signal names must match the
  Python workflows (`getCurrentState`, `alertMatchSignal`). This is marked `TODO(integration)` in `src/alerts/mod.rs`.
- **Blitzortung feed format.** The decoder is verified against Python-generated vectors. Real feed frames have not
  been checked.
