// A CarPlay build needs Apple's navigation capability in its signing profile, and
// the location prompt should say what CarPlay is for.
//
// The When-In-Use string is owned by the expo-location plugin (app.json), so the
// override has to replace that plugin's option rather than ios.infoPlist: the
// plugin's Info.plist mod runs after the config merge and would win otherwise.
const CARPLAY_LOCATION_PERMISSION =
  "Radar NG uses your location for nearby radar and turn-by-turn driving directions, including in CarPlay.";

const identity = require("./identity");

// Bundle ID, signing team, Android package and App Group come from identity.js
// (env with the upstream values as fallback), not app.json, so a fork sets them
// without editing tracked files. See FORKING.md.
function withIdentity(config) {
  return {
    ...config,
    ios: {
      ...config.ios,
      bundleIdentifier: identity.bundleId,
      appleTeamId: identity.appleTeamId,
      entitlements: {
        ...config.ios?.entitlements,
        "com.apple.security.application-groups": [identity.appGroup],
      },
    },
    android: { ...config.android, package: identity.androidPackage },
  };
}

// CI sets IOS_BUILD_RUN ("<run>.<attempt>") so each IPA gets a unique build number;
// prebuild copies ios.buildNumber into the widget and Watch targets.
function withBuildRun(config) {
  const run = process.env.IOS_BUILD_RUN;
  if (!run) return config;
  return { ...config, ios: { ...config.ios, buildNumber: `${config.ios?.buildNumber ?? "1"}.${run}` } };
}

module.exports = ({ config: input }) => {
  const config = withBuildRun(withIdentity(input));
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
      infoPlist: {
        ...config.ios?.infoPlist,
        UIBackgroundModes: [
          ...new Set([...(config.ios?.infoPlist?.UIBackgroundModes ?? []), "location", "audio"]),
        ],
      },
    },
  };
};

module.exports.withIdentity = withIdentity;
