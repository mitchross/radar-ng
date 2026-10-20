# Apple CarPlay review — Case-ID: 22361864

Apple's email requests screenshots illustrating how the app populates turn-by-turn
route guidance metadata (`CPManeuver`). It also states that navigation must be the
primary CarPlay service and that the app must supply a map through `CPMapTemplate`.
The user supplied this email; no email was sent by the agent.

## Captured evidence

**New capture after the user's request to change cities:** see
[Crossville radar screenshots](crossville/README.md). These show visible live-feed
radar returns along a Crossville-to-Knoxville route at regional map scale, with
tile-delivery diagnostics and visual inspection. They remain development-host
screenshots, not the actual CarPlay interface. The limitation below describes the
original Grand Rapids images kept here for the audit trail.

**Evidence limitation, verified September 22 after user review:** these images
show maneuver metadata in a separate iPhone test app. They do not demonstrate
visible precipitation or the actual CarPlay interface. The prior description of
this package as ready overstated what was verified. The radar timestamp confirms
manifest retrieval, not successful tile rendering; the capture harness does not
check tile delivery or rendering.

An audit of the server frame matching the displayed 12:08 AM timestamp
(`2026-09-22T04:08:07+00:00`) found the downtown Grand Rapids area transparent at
fallback zooms 6, 5, and 4. Zoom 7 returned 404. The parent tiles contain radar
returns elsewhere, but none in the inspected area (42.94–42.99 N,
85.69–85.65 W). This is consistent with the empty close-up; it does not establish
what the screenshot renderer actually received. See [tile audit](radar-tile-audit.json).
Before describing this as radar/CarPlay evidence, validate tile rendering in the
app and capture the actual CarPlay interface with visible real radar returns at
an appropriate location and map scale. Keep any GPS replay explicitly labelled.

`CarPlayReviewHost.swift` presents actual `CPManeuver` objects created by the same
`RadarManeuverFactory` used in the production CarPlay controller. It obtains a live
MapKit automobile route from Grand Rapids to Gerald R. Ford International Airport,
then replays route coordinates at 25-meter intervals. GPS replay is labelled.

The three planned captures show:

1. An upcoming maneuver about 300 m away, with instruction, arrow, trip estimate,
   next instruction, and the native route map (no visible precipitation).
2. The same maneuver about 40 m away, demonstrating decreasing live estimates.
3. The next maneuver after passing the first, demonstrating instruction advancement.

The JSON sidecar records actual instruction variants, dashboard variants, symbol
presence, initial leg estimates, updated maneuver estimates, and remaining trip
estimates. The harness fails rather than producing a passing report if it cannot
obtain a route, if required metadata is missing, if distance does not decrease, or
if the next maneuver fails to advance. It also checks stale GPS rejection and End Trip.

These are **development presentation screenshots**, not the CarPlay system UI.
They do not prove vehicle operation or Apple approval. The captions say so. Apple
may require native CarPlay system screenshots in a subsequent review. Do not remove
these labels or present these images as screenshots captured in a car.

## Verified September 22

- Captured on the iPhone 17 / iOS 27 simulator, after the user explicitly chose
  simulator capture. The physical iPhone remained unavailable.
- The production controller and review host share maneuver construction/estimates.
- Live MapKit geometry exposed an off-by-one error: step instructions describe the
  maneuver at the END of each step. Fixed both maneuver offsets and turn-symbol
  geometry; regression checks cover segment endpoints and left/right symbols.
- Screens 1 and 2 show the same left turn onto Crescent St NW at 300 m and 40 m.
  Localized display rounds these to 0.2 miles and 150 feet.
- Screen 3 shows advancement to the right turn onto Ionia Ave NW, 37.38 m away
  (displayed as 100 feet), after the Crescent turn.
- Native screenshot capture uses a run-specific handshake, preventing old results
  from a previous run from being accepted. Images are unedited device-framebuffer
  captures, each approximately 1 MB.
- Runtime checks passed for required metadata, decreasing distance, instruction
  advancement, stale GPS rejection, and End Trip. Native route regression checks
  and the review app Release build passed.
- The reply draft and three images were reviewed by the user, who identified the
  missing visible radar evidence. They are limited maneuver-test evidence.
  **No email sent.**
- CarPlay session and real-vehicle validation, including speech/audio interruptions
  and rerouting behavior, remain pending. These are not CarPlay system UI screenshots.

| Attachment | Demonstrates |
| --- | --- |
| [01-approaching-turn.png](01-approaching-turn.png) | Upcoming left-turn instruction and symbol |
| [02-near-turn.png](02-near-turn.png) | Decreasing distance for the same maneuver |
| [03-next-maneuver.png](03-next-maneuver.png) | Advancement to the subsequent right turn |

[Recorded values](maneuver-evidence.json) · [Runtime checks](review-result.txt) ·
[Email reply draft](reply-draft.txt)

## Capture on the selected device

From `frontend`, reconnect and unlock the physical iPhone, then run:

```sh
REVIEW_DEVICE_UDID=<physical-phone-udid> bash scripts/capture-carplay-review.sh
```

This builds a separate **Radar Nav Review** app under
`com.vanillax.radar-ng.carplay-review`, leaving the installed Radar NG app intact.
Xcode uses automatic development signing. It does not request the restricted
CarPlay entitlement, since the review host uses a normal iPhone window.

Only if the user chooses a simulator for this capture:

```sh
REVIEW_PLATFORM=simulator REVIEW_DEVICE_UDID=<booted-simulator-udid> \
  bash scripts/capture-carplay-review.sh
```

Output is printed by the runner. Inspect all three PNGs and `maneuver-evidence.json`,
confirm `review-result.txt` starts with PASS, then copy the reviewed evidence into
this directory. The existing captures have passed maneuver checks only, not radar
rendering or CarPlay integration checks. Address the evidence limitation above
before using this package to represent the completed feature. Any eventual reply
must preserve **Case-ID: 22361864**. Sending requires a separate explicit user instruction.
