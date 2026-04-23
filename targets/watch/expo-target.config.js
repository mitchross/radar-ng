/** @type {import('@bacons/apple-targets/app.plugin').ConfigFunction} */
module.exports = (config) => ({
  type: "watch",
  name: "StormScopeWatch",
  icon: "../../assets/images/icon.png",
  deploymentTarget: "10.0",
  colors: {
    $accent: "#8B7CFF",
  },
  entitlements: {},
});
