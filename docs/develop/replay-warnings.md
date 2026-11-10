# Historical warning cache

Issue [#282](https://github.com/mitchross/radar-ng/issues/282) adds an operator import and shared playhead selector.
It adds no HTTP routes, UI, notifications, worker schedules, or live data writes.

Build `radar`, create the output directory, then run:

```sh
radar import-replay-warnings greenfield-2024 /opt/data/cache/scratch/greenfield-warnings.geojson
radar import-replay-warnings rolling-fork-2023 /opt/data/cache/scratch/rolling-fork-warnings.geojson
```

Only these two cases are accepted. Their UTC windows and bounds match the
[replay design](design/famous-storm-replay.md#5-first-cases).
The output is one self-contained GeoJSON file. The importer replaces it atomically after all requests and validation succeed.
The parent directory must exist. A failed import preserves the previous file.
No manifest, live retention state, tiles, or cluster configuration changes.

## Upstream inputs and limits

The importer queries the [active-at-time snapshot](https://mesonet.agron.iastate.edu/geojson/sbw.py?help) at case start.
It then requests the [versioned interval API](https://mesonet.agron.iastate.edu/api/1/openapi.json)
with `begints`, `endts`, `only_new=false`, and `include_can=true`.
Snapshot timestamps use `polygon_begin`, `polygon_end`, `issue`, and `expire`.
Interval timestamps use `utc_polygon_begin`, `utc_polygon_end`, `utc_issue`, and `utc_expire`.

Discovery covers every office, then selects regional TO.W, SV.W, and FF.W events through polygon intersection.
Each regional seed event gets a separate office/phenomena interval query starting at its original issuance.
That query spans seven days and must include NEW. Expiry beyond that horizon fails the import.
The cache includes those full histories, even outside the replay window, and regional versions from the case interval.

Requests run sequentially. Each has a 45-second timeout and a 15-second connection timeout.
Limits are 32 seed events, 512 requests, 4,096 input versions, 16 MiB per response, and 64 MiB total downloads.
The output also has a 64 MiB cap. Redirects and missing bulletin segments fail the import.
Bulletin requests use `nolimit=true`, then select the exact WMO/AWIPS header and correction or repeat suffix.
Identical matches collapse. Conflicting exact matches fail the import.
Bulletin hashes cover the selected text. Request hashes cover the complete upstream response, including bundled products.
There is no public import endpoint or client request to IEM.

## Schema for the read-only API

Serve the completed file unchanged at a revision-pinned warnings URL in ticket #283.
Publish it with the matching manifest and tiles only after that revision is complete.
The warnings importer needs no code from ticket #281.

| Field | Meaning |
|---|---|
| `type`, `schema_version` | `FeatureCollection`, `1` |
| `case_id`, `start`, `end`, `bounds` | Case identity, UTC half-open window, `[west,south,east,north]` |
| `seed` | Normalized polygon features active at `start` |
| `features` | Normalized versions, including CAN history |
| `bulletins` | Product ID to `{url, sha256, text}` for each downloaded bulletin |
| `sources` | Request URLs and SHA-256 hashes of their response bytes |

Each feature has `type`, `id`, `geometry`, and `properties`.
Properties contain `year`, `wfo`, `phenomena`, `significance`, `eventid`, `event`, `status`, `product_id`,
`utc_polygon_begin`, `utc_polygon_end`, `is_emergency`, and `is_pds`.
Event identity is `(year, wfo, phenomena, significance, eventid)`.
Feature `id` joins that identity, product ID, and the source polygon begin with `/`.

Normalized validity starts at the later of source polygon begin and bulletin issue time.
It ends at the earliest polygon end, available event expiry, or next retained version's begin.
Duplicate begins choose the lexically greatest product ID, including its correction suffix.
Equal identities with conflicting content fail validation. Input order never decides the winner.
Ordinary zero-duration versions are discarded. CAN records remain as cancellation boundaries and never render.
EXP renders until actual expiry. Every interval uses `begin <= playhead < end`, including UTC midnight.

Emergency and PDS flags come only from the contemporaneous bulletin segment matching that VTEC event and action.
Lifetime maximum flags and snapshot labels never supply them. Missing or ambiguous segments fail validation.
A later correction cannot apply its wording before its bulletin issue minute.
If a NEW bulletin was issued after its polygon began, the import fails closed: IEM may have replaced the original polygon with a correction.
Recovering that missing original geometry is outside this importer; never publish an invented pre-correction gap.

The pure selector lives in [web/src/shared/replayWarnings.ts](../../web/src/shared/replayWarnings.ts).
The [app re-export](../../frontend/src/lib/replayWarnings.ts) uses the same implementation.
Both test runners consume [one fixture](../../web/src/shared/replayWarnings.fixture.json).
Rust normalization tests also compare their output against that fixture.
This groundwork does not deliver a phone feature. Ticket #284 needs both clients and native rebuilds.
