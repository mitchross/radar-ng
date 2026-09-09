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
so the iPhone, Watch, and widget all receive the new version and build number:

```sh
cd frontend
bunx expo prebuild --platform ios --no-install
cd ios
pod install
```

Use an installed, selected Xcode compatible with the app and accepted for the
intended distribution. The existing archive script defaults to an Xcode-beta path;
set `DEVELOPER_DIR` explicitly when invoking it on this Mac. Before uploading,
check App Store Connect/Play Console for builds made outside this repository and
advance the build counters further if necessary. This version bump does not
create or upload an archive, publish a store listing, or deploy backend images.

- [iPhone/Watch QA](ios-watch-qa-2026-09-08.md)
- [Widget/icon QA](widget-icon-qa-2026-09-09.md)
- [Apple platform setup](carplay-watch-setup.md)
