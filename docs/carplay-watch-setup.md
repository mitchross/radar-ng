# CarPlay + Apple Watch — Build & Sideload Guide

This doc covers building the CarPlay scene and watchOS target in `targets/` for **personal-device use only**. These features are NOT meant for App Store submission and use workarounds Apple does not officially bless.

## Prereqs (Mac only)

- macOS with Xcode 16+ installed
- iPhone with iOS 17+ (paired to your Apple Watch if testing Watch)
- Paid Apple Developer account (free tier blocks entitlements + limits sideload to 7 days)
- Carplay-equipped car OR CarPlay Simulator (Xcode → Window → Simulators → install iOS Simulator, then Features → External Displays → CarPlay)
- The repo cloned on Mac — you cannot compile iOS/watchOS from Linux

## Install deps

```bash
npm i -D @bacons/apple-targets
```

## Prebuild

From the project root on Mac:

```bash
npx expo prebuild --clean --platform ios
```

This will:

1. Generate `ios/` from `app.json`
2. Copy `targets/carplay/*.swift` into `ios/stormscope/CarPlay/` via `plugins/withCarPlayScene.js`
3. Add the `com.apple.developer.carplay-maps` entitlement to the main app's `.entitlements`
4. Register the `CPTemplateApplicationSceneSessionRoleApplication` scene in the main `Info.plist`
5. Generate a sibling watchOS target from `targets/watch/` via `@bacons/apple-targets`

Open the workspace:

```bash
open ios/stormscope.xcworkspace
```

## CarPlay — signing hack for personal use

Apple does not grant `com.apple.developer.carplay-maps` to non-navigation apps. The entitlement is already declared in the `.entitlements` file, but the standard provisioning flow will refuse to sign. Options, in order of preference:

### Option A — CarPlay Simulator (no hack needed)

Fully supported, no entitlement required at sign time.

1. In Xcode, run the app on a regular iOS Simulator
2. Simulator menu → **Features → External Displays → CarPlay**
3. A second window opens showing the CarPlay UI with `RadarMapController`
4. This is good enough for 95% of development

### Option B — Sideload to real device/car (the hack)

You need to bypass Xcode's entitlement validation. Two known paths:

**B1. Manual re-sign post-build (most reliable):**

```bash
# Build for device with auto-signing (will strip the carplay entitlement)
xcodebuild -workspace ios/stormscope.xcworkspace \
  -scheme stormscope \
  -configuration Release \
  -destination 'generic/platform=iOS' \
  -archivePath build/stormscope.xcarchive archive

# Copy entitlements with carplay-maps back in
cat > /tmp/entitlements-patched.plist <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>application-identifier</key><string>YOUR_TEAMID.com.anonymous.stormscope</string>
  <key>com.apple.developer.team-identifier</key><string>YOUR_TEAMID</string>
  <key>com.apple.developer.carplay-maps</key><true/>
  <key>get-task-allow</key><true/>
</dict>
</plist>
EOF

# Re-sign the .app inside the archive
codesign --force --sign "Apple Development: your@email.com" \
  --entitlements /tmp/entitlements-patched.plist \
  --deep --preserve-metadata=identifier,requirements,flags,runtime \
  build/stormscope.xcarchive/Products/Applications/stormscope.app

# Install to connected device
ios-deploy --bundle build/stormscope.xcarchive/Products/Applications/stormscope.app
```

Success rate: iOS will load the app. When CarPlay connects, the scene delegate fires because the entitlement is present in the embedded provisioning profile's associated app (iOS checks the app's signed entitlements, not the Apple dev portal).

**B2. AltStore / TrollStore:**

If B1 refuses, TrollStore on supported iOS versions can install arbitrary entitlements. This is device-dependent — out of scope for this doc.

### Option C — Give up on real car, use simulator only

Acceptable if your car supports wireless CarPlay — you can dev against Simulator and just enjoy the app on the phone itself.

## Watch — no hacks needed

The Watch target has no gated entitlements. Standard flow:

1. Xcode → scheme dropdown → `StormScopeWatch`
2. Select your paired Apple Watch (must be on same Wi-Fi or paired iPhone nearby)
3. Cmd+R
4. First run: allow location prompt on watch

If it fails to install on device, switch the scheme's destination to a Watch Simulator and confirm it builds, then retry device.

## What gets rendered

**CarPlay (`RadarMapController`):**

- `MKMapView` with `mutedStandard` basemap
- `MKTileOverlay` hitting Iowa State's public NEXRAD tile cache (`mesonet.agron.iastate.edu/...nexrad-n0q-0/{z}/{x}/{y}.png`, TMS Y-flipped)
- User location pin
- Top bar: re-center, refresh (bumps cache key to force tile reload), opacity cycle (40/60/80/100%)
- Auto-refresh every 5 min
- No animation / timeline — CarPlay UI guidelines prohibit rich animation

**Watch (`ContentView`):**

- Current temp + condition (large Thin font, SF Symbols)
- Alert badge at top if any NWS active alert
- 60-minute nowcast bar chart from `minutely_15.precipitation`
- 12-hour scroll row
- 5-day list
- Pull-to-refresh
- Hits `https://radar-ng-api.vanillax.me/api/forecast/{lat}/{lon}` + NWS alerts

## Troubleshooting

- **"Provisioning profile doesn't include com.apple.developer.carplay-maps"**: you hit this when signing normally. Use Option B1 post-archive re-sign, or switch to CarPlay Simulator.
- **Watch app shows "NSURLErrorDomain"**: watchOS 10+ requires `NSAppTransportSecurity` for non-https if you point at `http://` — your server is https already so you're fine.
- **Tile overlay blank on CarPlay**: check `RadarTileOverlay.url(forTilePath:)` — the TMS Y flip is required for IEM. Hit the URL in a browser to verify.
- **Scene delegate never fires on real car**: the entitlement is missing from the signed app. Verify with `codesign -d --entitlements - /path/to/stormscope.app`.

## Files

```
targets/
├── carplay/
│   ├── RadarCarPlaySceneDelegate.swift  — CPTemplateApplicationSceneDelegate
│   ├── RadarMapController.swift         — CPMapTemplate + MKMapView host
│   ├── RadarTileOverlay.swift           — MKTileOverlay (IEM NEXRAD, TMS)
│   ├── RadarLocationManager.swift       — CoreLocation wrapper
│   └── RadarAPI.swift                   — backend URLs
└── watch/
    ├── expo-target.config.js            — @bacons/apple-targets config
    ├── RadarWatchApp.swift              — @main SwiftUI app
    ├── WatchStore.swift                 — ObservableObject, fetches + location
    ├── WatchAPI.swift                   — fetchForecast, fetchAlerts, Codable types
    ├── ContentView.swift                — all watch views
    └── Info.plist

plugins/withCarPlayScene.js              — copies carplay/ into main app, adds entitlement, adds scene to Info.plist
```
