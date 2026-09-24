const { withStringsXml, AndroidConfig } = require("@expo/config-plugins");

const APP_LABEL = "Radar NG";

// The launcher label is generated from expo.name, but expo.name also names the
// native projects: it renames ios/radarng to ios/RadarNG on a clean prebuild and
// becomes the Android settings.gradle rootProject. Every build script and doc
// that references the `radarng` scheme or workspace would break, so set only the
// launcher string here and leave expo.name as the project name.
module.exports = (config) =>
  withStringsXml(config, (c) => {
    c.modResults = AndroidConfig.Strings.setStringItem(
      [AndroidConfig.Resources.buildResourceItem({ name: "app_name", value: APP_LABEL })],
      c.modResults,
    );
    return c;
  });
