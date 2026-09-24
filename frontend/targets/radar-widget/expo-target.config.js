/** @type {import('@bacons/apple-targets/app.plugin').Config} */
module.exports = {
  type: "widget",
  name: "RadarWidget",
  displayName: "Radar NG",
  bundleIdentifier: ".radar-widget",
  deploymentTarget: "26.0",
  frameworks: ["CoreLocation"],
  // Reads the app's shared server, palette and location (RadarShared.swift).
  entitlements: {
    "com.apple.security.application-groups": ["group.com.vanillax.radar-ng"],
  },
};
