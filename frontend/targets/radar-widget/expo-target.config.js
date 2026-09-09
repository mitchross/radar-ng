/** @type {import('@bacons/apple-targets/app.plugin').Config} */
module.exports = {
  type: "widget",
  name: "RadarWidget",
  displayName: "Radar NG",
  bundleIdentifier: ".radar-widget",
  deploymentTarget: "26.0",
  frameworks: ["MapKit", "CoreLocation"],
  entitlements: {},
};
