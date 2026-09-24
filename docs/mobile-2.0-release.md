# Radar NG 2.0.0

Mobile version: **2.0.0**. iOS build: **3**. Android version code: **3**.
These advance the checked-in mobile versions; backend image tags are independent.

## Release notes

- Refreshed radar icon and improved native iPhone controls and layouts.
- More reliable radar playback, saved settings, and forecast presentation.
- Reworked Apple Watch radar maps, location handling, refresh behavior, and controls.
- New Nearby Radar widget for iPhone, StandBy, and the iOS 26+ CarPlay widget screen.
  Shows a radar snapshot and observation time, with explicit unavailable and older-data states.
- Updated Expo/React Native dependencies and performance improvements.

## Release assessment

The scope warrants a 2.0 mobile milestone. The recommended first distribution is
TestFlight/internal testing. Native iPhone, widget and Watch builds and simulator
QA passed on the preceding changes; see the linked evidence below.

Physical iPhone/Watch testing, actual CarPlay layout/refresh cadence, and Android
native QA remain outstanding. The Android icon currently has a new adaptive
foreground but no dedicated themed monochrome mask. Full-screen CarPlay navigation
and animated CarPlay radar are not included. Do not advertise them as 2.0 features.

## Build preparation

The Expo config is the version source. Regenerate native projects before archiving
so the iPhone, Watch, and widget all receive the new version and build number.
The release order is **clean prebuild → pod install → preflight → archive**:

```sh
cd frontend
bunx expo prebuild --platform ios --clean --no-install
cd ios
pod install
cd ..
bash scripts/preflight-ios-release.sh          # add RADAR_CARPLAY=1 for a CarPlay build
bash scripts/build-ios-standalone.sh           # runs the preflight again, then archives
```

The preflight fails on the mixed native trees that have shipped before: CarPlay
background modes or the `carplay-maps` entitlement in a build that is not a
CarPlay build, `NSLocationAlways*`/`NSFaceID*`/`NSMotion*` permission strings,
version drift between `app.json` and the Watch/widget targets, and a beta Xcode.
The archive script defaults `DEVELOPER_DIR` to `/Applications/Xcode.app/Contents/Developer`
(Xcode 27.0 release). Archiving with a beta needs both an explicit `DEVELOPER_DIR`
and `ALLOW_BETA=1`.

Use an installed, selected Xcode compatible with the app and accepted for the
intended distribution. Before uploading, check App Store Connect/Play Console for
builds made outside this repository and advance the build counters further if
necessary. This version bump does not create or upload an archive, publish a store
listing, or deploy backend images.

- [iPhone/Watch QA](ios-watch-qa-2026-09-08.md)
- [Widget/icon QA](widget-icon-qa-2026-09-09.md)
- [Apple platform setup](carplay-watch-setup.md)
