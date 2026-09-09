# Apple Watch and CarPlay development

The Watch app and experimental CarPlay map scene are native Swift sources under
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
The current `RadarMapController` is an experimental radar map, not a navigation app.
It still uses Iowa Mesonet tiles and has not been verified in an actual car.

A **radar widget** is the supported first path for displaying radar in the user's
car without turning Radar NG into a full navigation app. iOS 26+ CarPlay can show
`.systemSmall` widgets from ordinary iPhone apps. A widget can show a timestamped
radar snapshot; system-scheduled widget refreshes are not a continuous animation.
This widget is a followup, not an implemented feature in this change. Watch/iPhone
quality work takes priority per the current user direction.

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
