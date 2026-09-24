# Apple Watch and CarPlay development

The Watch app and CarPlay navigation implementation are native Swift sources under
`frontend/targets/`. Expo prebuild registers them in `frontend/ios/radarng.xcworkspace`.
Keep both the iPhone and CarPlay scene declarations: removing the iPhone scene
breaks React Native window attachment.

## Apple Watch

From `frontend`, install the locked dependencies and generate the native project:

```sh
bun install --frozen-lockfile
bunx expo prebuild --platform ios --no-install
```

Open `ios/radarng.xcworkspace`, select `radar-ngWatch`, and choose a Watch simulator
or your paired Watch. Use normal Apple development signing for a physical Watch.
The Watch target has no CarPlay entitlement requirement.

The radar page uses a native MapKit snapshot plus the latest self-hosted MRMS
raster tiles. The Digital Crown and +/- buttons change zoom; refresh fetches a new
manifest. Only tiles intersecting the screen are loaded. Swiping up opens a native
SwiftUI forecast page. Its hourly strip starts at the forecast's current hour;
its next-hour precipitation uses four upcoming 15-minute samples, not the first
60 samples from midnight. Missing data and location fallback are explicitly labeled.
Forecast and radar use the Radar API; Watch alerts still use NWS directly.
Location refreshes reject stale/inaccurate fixes, discard weather responses for a
previous location, and label the fallback or last-known location. Missing high-zoom
radar tiles fall back to a cropped lower-resolution tile from the same frame.
Loading/failure states are visible; radar older than 15 minutes is hidden. Expired
alerts are omitted and failed alert requests are labelled unavailable.

Native UI tests and launch measurements:

```sh
WATCH_SIMULATOR_UDID=<watch-uuid> \
IOS_SIMULATOR_APP_PATH=/path/to/Release-iphonesimulator/radarng.app \
bash scripts/test-watch-simulator.sh
```

For a paired Watch simulator, the companion app must be installed on its paired
iPhone. Otherwise the paired-device machinery can remove the Watch app during tests.
`IOS_SIMULATOR_APP_PATH` installs it; omit this variable if it is already installed.

The runner adds a UI-test target to the generated project and runs the checked-in
Watch tests. It does not modify app provisioning or deploy to a physical device.
See [the September QA report](ios-watch-qa-2026-09-08.md) for tested devices and limits.

## CarPlay: real car requirements

The previous guide and `carplay-resign.sh` incorrectly claimed that adding an
entitlement after archiving bypasses provisioning checks. **It does not.** iOS
validates signed entitlements against the embedded provisioning profile. Developer
Mode and a paid Apple Developer account do not grant restricted CarPlay capabilities.
The old command now stops with an explanation instead of producing a misleading build.

For a full CarPlay app, request Apple's category-specific capability, enable it for
the app identifier, regenerate the development profile, and include the matching
entitlement in the app. `com.apple.developer.carplay-maps` is for navigation apps
with route guidance; Apple's driving-task category does not permit custom maps.
The current implementation includes destination search, MapKit driving routes,
route alternatives, turn instructions, voice prompts, GPS-loss handling, rerouting,
and a Dashboard map sharing the active journey. Radar uses Radar NG's timestamped
MRMS tiles; missing high-zoom tiles fall back to the same frame's parent tile.
This is development code, not yet verified in an actual CarPlay session or vehicle.
Travel times during a journey are approximate, derived from MapKit's route estimate.

Ordinary phone/Watch builds intentionally omit the restricted entitlement. After
Apple grants Navigation to this team and bundle identifier, generate a CarPlay build:

```sh
RADAR_CARPLAY=1 bunx expo prebuild --platform ios --no-install
python3 scripts/check-carplay-profile.py /path/to/profile.mobileprovision
```

The profile must contain `com.apple.developer.carplay-maps = true`, match
`FVU6RGL532.com.vanillax.radar-ng`, and be unexpired. Checking a capability or adding
the entitlement locally cannot grant Apple's approval. To return to ordinary
signing, run prebuild without `RADAR_CARPLAY`; the plugin removes any previously
generated maps entitlement. Keep both iPhone and CarPlay scene declarations.

The September 21 request is prepared in the signed-in browser, category Navigation,
with an honest planned-feature description and labelled iPhone development-preview
images. It awaits the account owner's agreement/submission unless separately
confirmed. See [the request and validation record](carplay-request-2026-09-21.md).

A **radar widget** is another supported path for displaying radar in the user's
car without turning Radar NG into a full navigation app. iOS 26+ CarPlay can show
`.systemSmall` widgets from ordinary iPhone apps. A widget can show a timestamped
radar snapshot; system-scheduled widget refreshes are not a continuous animation.
The native `RadarWidget` target now implements this first step. It uses the
self-hosted MRMS manifest and classic radar tiles, composited over a MapKit
snapshot. The widget shows the frame timestamp (not the download time), requests an
update after ten minutes, and schedules an older-data label after fifteen minutes.
WidgetKit decides when to refresh; this is not a live feed or continuously tracked
driving position. The location dot belongs to the sampled snapshot.

Open Radar NG on the iPhone and allow location access first. Then add **Nearby
Radar** from the iPhone widget gallery, or from **Settings → General → CarPlay →
your car → Widgets** on iOS 26+. Accept the separate system prompt allowing Radar
NG's widgets to use your location. The widget is `.systemSmall`, uses a removable
background, and does not request a CarPlay entitlement or App Group. Location is
requested by the extension through WidgetKit's supported location authorization.
If location or any required tile is unavailable, it shows an explicit message
instead of presenting a partial map as clear weather. Coverage is continental US.

For private testing, build and install the containing iPhone app with normal Apple
development signing. No public App Store release is required. The extension must
also be signed for the same development team. A simulator build cannot be installed
on a physical iPhone.

Run the isolated native rendering test (real API, MapKit, timestamp parsing,
coverage rejection, and small-size/error presentation):

```sh
IOS_SIMULATOR_UDID=<booted-iphone-uuid> bash scripts/test-widget-simulator.sh
```

### Next: animated radar

The requested next step is animated radar. WidgetKit supports short transitions
when data changes, with a maximum duration of two seconds; it does not provide a
supported continuous radar playback loop. Do not generate per-second timelines
or describe repeated widget refresh requests as animation. Live Activities have
the same documented animation-duration limit.

Investigate playback in the full native CarPlay map separately, including Apple's
navigation entitlement and permitted driving presentation. Approval of a map app
does not establish approval of a looping weather animation. First validate frame
caching, frame timestamps, pause/resume, and rendering in the prototype simulator,
then confirm the CarPlay presentation with Apple before promising it in the car.

- [Apple: widgets in CarPlay without a CarPlay app](https://developer.apple.com/videos/play/wwdc2025/216/)
- [Apple: widget location access](https://developer.apple.com/documentation/widgetkit/accessing-location-information-in-widgets/)
- [Apple: widget animation limits](https://developer.apple.com/documentation/widgetkit/animating-data-updates-in-widgets-and-live-activities)

## CarPlay simulators

Apple's standalone CarPlay Simulator (Additional Tools for Xcode / Device Hub)
connects to a physical iPhone. It does not bypass that phone's provisioning checks.
Xcode's iOS Simulator also offers I/O → External Displays → CarPlay for supported
runtimes. Registering a scene alone does not make the app eligible for CarPlay;
follow Apple's entitlement setup. Neither kind of simulator proves operation in a car.

## References

- [CarPlay Developer Guide, June 2026](https://developer.apple.com/download/files/CarPlay-Developer-Guide.pdf): widgets, category rules, and entitlements.
- [Requesting CarPlay entitlements](https://developer.apple.com/documentation/carplay/requesting-carplay-entitlements).
- [Apple entitlement troubleshooting](https://developer.apple.com/library/archive/technotes/tn2415/_index.html).
- [Setting up Watch tests](https://developer.apple.com/documentation/watchos-apps/setting-up-tests-for-your-watchos-app).

## Physical iPhone: integrity could not be verified

On September 21, an exported release-testing IPA was cryptographically valid but
its iPhone and widget profiles listed the previous iPhone's UDID, not the currently
connected phone. Both phones had the same user-visible name. Xcode reported an
integrity/install failure rather than identifying the device mismatch in its first
error line. Changing the app's CarPlay entitlement did not refresh those old exports.

Use `xcrun devicectl list devices` to identify the connected **physical** phone by
UDID, then build for that explicit destination so automatic signing can register it:

```sh
xcodebuild -workspace ios/radarng.xcworkspace -scheme radarng \
  -configuration Release -destination 'platform=iOS,id=PHONE_UDID' \
  -allowProvisioningUpdates -allowProvisioningDeviceRegistration build
```

Re-export the archive with refreshed profiles (`debugging` for direct local testing,
`release-testing` for ad hoc distribution). The previous IPA remains invalid for the
new phone. Inspect `embedded.mobileprovision` with `security cms -D -i` and confirm
that the iPhone app and each iOS extension include the target phone in
`ProvisionedDevices`. Watch provisioning is separate; validate against the Watch
when diagnosing a Watch-only installation failure. Never remove the installed app
or its data merely to address a profile mismatch.
