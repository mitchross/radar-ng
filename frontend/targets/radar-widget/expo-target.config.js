const { appGroup } = require("../../identity");

/** @type {import('@bacons/apple-targets/app.plugin').Config} */
module.exports = {
  type: "widget",
  name: "RadarWidget",
  displayName: "Radar NG",
  bundleIdentifier: ".radar-widget",
  deploymentTarget: "26.0",
  frameworks: ["CoreLocation"],
  // Reads the app's shared server, palette and location (RadarShared.swift).
  // The App Group comes from identity.js, like the app's own entitlement.
  entitlements: {
    "com.apple.security.application-groups": [appGroup],
  },
};
