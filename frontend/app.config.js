// A CarPlay build needs Apple's navigation capability in its signing profile, and
// the location prompt should say what CarPlay is for.
//
// The When-In-Use string is owned by the expo-location plugin (app.json), so the
// override has to replace that plugin's option rather than ios.infoPlist: the
// plugin's Info.plist mod runs after the config merge and would win otherwise.
const CARPLAY_LOCATION_PERMISSION =
  "Radar NG uses your location for nearby radar and turn-by-turn driving directions, including in CarPlay.";

module.exports = ({ config }) => {
  if (process.env.RADAR_CARPLAY !== "1") return config;

  const plugins = (config.plugins ?? []).map((entry) => {
    if (Array.isArray(entry) && entry[0] === "expo-location") {
      return ["expo-location", { ...entry[1], locationWhenInUsePermission: CARPLAY_LOCATION_PERMISSION }];
    }
    return entry;
  });

  return {
    ...config,
    plugins,
    ios: {
      ...config.ios,
      entitlements: {
        ...config.ios?.entitlements,
        "com.apple.developer.carplay-maps": true,
      },
      infoPlist: {
        ...config.ios?.infoPlist,
        UIBackgroundModes: [
          ...new Set([...(config.ios?.infoPlist?.UIBackgroundModes ?? []), "location", "audio"]),
        ],
      },
    },
  };
};
