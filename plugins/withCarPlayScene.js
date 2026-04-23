const { withEntitlementsPlist, withDangerousMod, withXcodeProject } = require("@expo/config-plugins");
const fs = require("fs");
const path = require("path");

const CARPLAY_SRC = "targets/carplay";
const CARPLAY_FILES = [
  "RadarCarPlaySceneDelegate.swift",
  "RadarMapController.swift",
  "RadarTileOverlay.swift",
  "RadarLocationManager.swift",
  "RadarAPI.swift",
];

function withCarPlayEntitlement(config) {
  return withEntitlementsPlist(config, (c) => {
    c.modResults["com.apple.developer.carplay-maps"] = true;
    return c;
  });
}

function withCarPlayFiles(config) {
  return withDangerousMod(config, [
    "ios",
    async (c) => {
      const projectRoot = c.modRequest.projectRoot;
      const iosRoot = c.modRequest.platformProjectRoot;
      const iosAppName = c.modRequest.projectName;
      const destDir = path.join(iosRoot, iosAppName, "CarPlay");
      fs.mkdirSync(destDir, { recursive: true });
      for (const f of CARPLAY_FILES) {
        const src = path.join(projectRoot, CARPLAY_SRC, f);
        const dst = path.join(destDir, f);
        if (fs.existsSync(src)) fs.copyFileSync(src, dst);
      }
      return c;
    },
  ]);
}

function withCarPlayPbxproj(config) {
  return withXcodeProject(config, (c) => {
    const proj = c.modResults;
    const appName = c.modRequest.projectName;
    const groupKey = proj.pbxCreateGroup("CarPlay", `${appName}/CarPlay`);
    const mainGroup = proj.getFirstProject().firstProject.mainGroup;
    proj.addToPbxGroup(groupKey, mainGroup);
    const target = proj.getFirstTarget().uuid;
    for (const f of CARPLAY_FILES) {
      proj.addSourceFile(`${appName}/CarPlay/${f}`, { target }, groupKey);
    }
    return c;
  });
}

module.exports = (config) => {
  config = withCarPlayEntitlement(config);
  config = withCarPlayFiles(config);
  config = withCarPlayPbxproj(config);
  return config;
};
