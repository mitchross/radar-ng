// A CarPlay build needs Apple's navigation capability in its signing profile.
module.exports = ({ config }) => {
  if (process.env.RADAR_CARPLAY !== "1") return config;
  return {
    ...config,
    ios: {
      ...config.ios,
      entitlements: {
        ...config.ios?.entitlements,
        "com.apple.developer.carplay-maps": true,
      },
      infoPlist: {
        ...config.ios?.infoPlist,
        NSLocationWhenInUseUsageDescription:
          "Radar NG uses your location for nearby radar and turn-by-turn driving directions, including in CarPlay.",
        UIBackgroundModes: [
          ...new Set([...(config.ios?.infoPlist?.UIBackgroundModes ?? []), "location", "audio"]),
        ],
      },
    },
  };
};
