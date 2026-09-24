# Radar NG CarPlay request — September 21, 2026

App: Radar NG (`com.vanillax.radar-ng`), team `FVU6RGL532`.
Category: Navigation (turn-by-turn directions).
Request: https://developer.apple.com/contact/request/carplay/

## Prepared request

Radar NG is an iPhone weather radar app with an Apple Watch companion. We are
developing a navigation experience for CarPlay: destination search, driving route
previews, turn-by-turn instructions, spoken guidance, travel estimates, and
rerouting, with timestamped precipitation radar as supplementary map context.
Navigation will be the primary CarPlay task. The native implementation is in
development and has not been tested in a vehicle or approved for CarPlay. The
request seeks the Navigation entitlement to complete development and device validation.

Planned features use CPMapTemplate, CPSearchTemplate, CPTrip route previews,
CPNavigationSession guidance, and a Dashboard navigation scene. Standard CarPlay
templates host controls; custom views contain map content only. Map controls include
recenter, zoom, and pan. Guidance includes spoken directions, mute, end-trip,
GPS-loss handling, and off-route recalculation. Radar is supplementary and uses the
latest observed frame; it does not promise weather-aware routing or continuous animation.
Interruption handling, GPS loss, background navigation, cancellation, and vehicle
presentation still need device validation before release.

The optional App Store URL is blank. Three uploaded images are explicitly labelled
**development preview / CarPlay approval pending / simulated GPS**. They show the
shared MapKit search and routing implementation in an iPhone test host, not actual
CarPlay templates. Original JPGs are in `/tmp/radar-carplay-qa/` on the development Mac.

The user subsequently supplied Apple's response for **Case-ID: 22361864**, confirming
that the request was submitted and is under review. Apple asks for screenshots
illustrating how Radar NG populates turn-by-turn guidance metadata (`CPManeuver`).
This is a request for more information, not entitlement approval. See
[the follow-up package](carplay-review-22361864/README.md).

## Validation and limits

- Full Release iOS/Watch/widget build succeeded before the final Watch and tile-fallback edits.
- Final native CarPlay and Watch sources pass device-SDK Swift type checking.
- Local Swift regression checks cover route projection, loop protection, GPS validity,
  timestamp parsing, expired radar/alerts, tile bounds, and forecast presentation.
- JavaScript config regression covers normal provisioning and removing a previously
  enabled CarPlay entitlement. The existing frontend suite passed before these native edits.
- The earlier live MapKit harness reached search, preview, simulated route progression,
  stale-GPS rejection and cancellation, but failed its final radar tile check. The live
  service returned 404 at zoom 7 and 200 for the same frame at zoom 6/5. Both native
  maps now fall back to lower-resolution tiles. This fallback has been type checked;
  the simulator harness was not rerun because the user explicitly stopped simulator work.
- No signed CarPlay device build, real vehicle session, or final Watch UI run is claimed.
- Current generated iOS entitlements omit maps to unblock ordinary device signing.
  Apple approval and an updated provisioning profile remain required for CarPlay.
