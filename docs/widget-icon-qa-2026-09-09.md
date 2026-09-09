# Radar widget and icon QA — September 9, 2026

## Delivered

- Native `RadarWidget` extension, supported on the small Home Screen/StandBy/CarPlay
  widget surface. No restricted CarPlay capability or App Group is requested.
- Snapshot of current-location MRMS radar over MapKit. The frame timestamp and
  older-data state distinguish the snapshot from live playback. Location denial,
  location timeout, missing tiles, and unsupported coverage have explicit states.
- Bounded location/network/map work. At most four intersecting tiles download
  concurrently with the map snapshot; rendering uses a 256×190 image at 1× scale.
- New generated mint radar mark on ink, replacing the detailed grid artwork.
  The clean opaque 1024×1024 master feeds the Apple Icon Composer source and Watch
  icon. Android gets a padded adaptive foreground and a smaller splash asset.
  The old Android monochrome mask was removed with the obsolete identity; a new
  dedicated themed mask belongs in the pending Android visual pass.

## Verification

- Release build of the containing iPhone app, embedded WidgetKit extension and
  Watch app succeeds with Xcode 27 beta 6. Build by destination, without forcing
  `-sdk iphonesimulator` on the Watch target.
- Isolated native simulator host passes real API/MapKit rendering, ISO timestamps
  with/without fractional seconds, malformed timestamps, and unsupported coverage.
  Observed snapshot fetch/render times were 0.67–0.91 seconds on this Mac; these
  measurements exclude location acquisition and are not physical-device benchmarks.
- Maestro opened the Home Screen widget gallery, searched Radar NG, and added
  Nearby Radar. The new icon is visible in the gallery and Home Screen.
- Confirmed the separate system permission prompt for widget location access.
  After allowing it, the installed extension displayed real radar. Some system
  gallery controls are absent from Maestro's accessibility tree, so their observed
  screen coordinates were used for those taps.
- Public Expo configuration resolves; the master icon is 1024×1024 with no alpha.

Screenshots: [native rendering states](qa/2026-09-09/widget-states.png),
[gallery and new icon](qa/2026-09-09/widget-gallery-new-icon.png),
[installed widget](qa/2026-09-09/widget-home-screen.png).

## Remaining device checks

The physical iPhone was unavailable to the Mac. Actual CarPlay display layout,
refresh cadence during a drive, physical Watch icon appearance, and Android native
appearance remain unverified. Install a normally signed development build on the
iPhone, launch it, grant app and widget location permission, and add Nearby Radar
in the car's widget settings. No App Store publication is necessary.

Continuous animated radar is explicitly the next requested feature. WidgetKit's
two-second update transitions are not a supported playback loop. The next work
must separately assess full CarPlay map playback and Apple's navigation/category
requirements; this PR does not claim animation works in a car.
