// Xcode 26 enables `ENABLE_USER_SCRIPT_SANDBOXING = YES` by default, which
// blocks Expo's `[Expo] Configure project` build phase from writing into the
// app source tree (ExpoModulesProvider.swift, .entitlements). Flip it off on
// every target so prebuild output is buildable without manually editing the
// pbxproj after each prebuild.
const { withXcodeProject } = require("@expo/config-plugins");

module.exports = (config) => {
  return withXcodeProject(config, (c) => {
    const proj = c.modResults;
    const configurations = proj.pbxXCBuildConfigurationSection();
    for (const key of Object.keys(configurations)) {
      const cfg = configurations[key];
      if (typeof cfg !== "object" || !cfg.buildSettings) continue;
      cfg.buildSettings.ENABLE_USER_SCRIPT_SANDBOXING = "NO";
    }
    return c;
  });
};
