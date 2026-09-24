const fs = require("fs");
const path = require("path");
const { withAndroidManifest, withDangerousMod } = require("@expo/config-plugins");

// Users can point the app at a server on their LAN over plain http, so
// cleartext stays allowed. Android ignores `usesCleartextTraffic` for apps
// targeting API 38+, so the permission lives in a Network Security Config.
const NETWORK_SECURITY_XML = `<?xml version="1.0" encoding="utf-8"?>
<network-security-config>
  <!-- Self-hosted servers on a home network are often plain http. -->
  <base-config cleartextTrafficPermitted="true">
    <trust-anchors>
      <certificates src="system" />
    </trust-anchors>
  </base-config>
</network-security-config>
`;

const CONFIG_REF = "@xml/network_security_config";

function applyNetworkSecurityConfig(manifest) {
  const app = manifest.manifest.application?.[0];
  if (!app) throw new Error("AndroidManifest.xml has no <application>");
  app.$ = { ...app.$, "android:networkSecurityConfig": CONFIG_REF };
  delete app.$["android:usesCleartextTraffic"];
  return manifest;
}

const withAndroidNetworkSecurity = (config) => {
  config = withAndroidManifest(config, (c) => {
    c.modResults = applyNetworkSecurityConfig(c.modResults);
    return c;
  });
  return withDangerousMod(config, [
    "android",
    async (c) => {
      const dir = path.join(c.modRequest.platformProjectRoot, "app/src/main/res/xml");
      await fs.promises.mkdir(dir, { recursive: true });
      await fs.promises.writeFile(path.join(dir, "network_security_config.xml"), NETWORK_SECURITY_XML);
      return c;
    },
  ]);
};

module.exports = withAndroidNetworkSecurity;
module.exports.applyNetworkSecurityConfig = applyNetworkSecurityConfig;
module.exports.NETWORK_SECURITY_XML = NETWORK_SECURITY_XML;
