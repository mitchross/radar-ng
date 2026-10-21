// The app's identity: who signs it, what it is called on the stores, and which
// servers a fresh install talks to. This is the one place those values live.
//
// Every value reads an environment variable (shell or frontend/.env) and falls
// back to the upstream project's own value, so an upstream build needs no setup
// and a fork changes them without editing code. FORKING.md has the checklist.
//
// Read by app.config.js (bundle ID, team, package, App Group), the widget's
// expo-target.config.js, the build and QA scripts, and
// scripts/sync-native-identity.js, which copies the Swift-side values into the
// native sources (Swift can't read env at runtime, so those are kept in sync by
// that script and checked by __tests__/plugins/identity.test.js).

const env = (name, fallback) => {
  const value = process.env[name];
  return value && value.trim() ? value.trim() : fallback;
};

const stripSlash = (url) => url.replace(/\/+$/, "");

const bundleId = env("RADAR_BUNDLE_ID", "com.vanillax.radar-ng");

module.exports = {
  /** iOS bundle identifier. The widget and Watch derive theirs from it (`.radar-widget`, `.watch`). */
  bundleId,
  /** App Group shared by the app, widget and CarPlay (RadarShared.swift). */
  appGroup: env("RADAR_APP_GROUP", `group.${bundleId}`),
  /** Android applicationId. */
  androidPackage: env("RADAR_ANDROID_PACKAGE", "com.vanillax.radarng"),
  /** Apple Developer team that signs the build. */
  appleTeamId: env("RADAR_APPLE_TEAM_ID", "FVU6RGL532"),
  /** The stack a fresh install points at (Settings → Advanced → Stack URL). Also read by src/lib/constants.ts. */
  defaultServerUrl: stripSlash(env("EXPO_PUBLIC_RADAR_DEFAULT_URL", "https://radar-ng-api.vanillax.me")),
  /** tileserver-gl raster endpoint for the Watch and widget basemap images. */
  basemapRasterUrl: stripSlash(env("RADAR_BASEMAP_RASTER_URL", "https://maps.vanillax.me/raster")),
};
