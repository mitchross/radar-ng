const fs = require("fs");
const os = require("os");
const path = require("path");

jest.mock("@expo/config-plugins", () => {
  const actual = jest.requireActual("@expo/config-plugins");
  return {
    ...actual,
    withAndroidManifest: (config, mod) => ({ ...config, manifestMod: mod }),
    withDangerousMod: (config, [platform, mod]) => ({ ...config, dangerousMod: { platform, mod } }),
  };
});

const plugin = require("../../plugins/withAndroidNetworkSecurity");
const { applyNetworkSecurityConfig, NETWORK_SECURITY_XML } = plugin;

test("points the application at the config and drops the ignored attribute", () => {
  const manifest = {
    manifest: {
      application: [{ $: { "android:name": ".MainApplication", "android:usesCleartextTraffic": "true" } }],
    },
  };
  const app = applyNetworkSecurityConfig(manifest).manifest.application[0].$;
  expect(app["android:networkSecurityConfig"]).toBe("@xml/network_security_config");
  expect(app["android:usesCleartextTraffic"]).toBeUndefined();
  expect(app["android:name"]).toBe(".MainApplication");
});

test("keeps LAN http working: cleartext permitted, system CAs trusted", () => {
  expect(NETWORK_SECURITY_XML).toContain('<base-config cleartextTrafficPermitted="true">');
  expect(NETWORK_SECURITY_XML).toContain('<certificates src="system" />');
  expect(NETWORK_SECURITY_XML).not.toContain('src="user"');
});

test("writes the XML into the Android res/xml directory", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "nsc-"));
  const { dangerousMod } = plugin({});
  expect(dangerousMod.platform).toBe("android");
  await dangerousMod.mod({ modRequest: { platformProjectRoot: root } });
  const written = fs.readFileSync(path.join(root, "app/src/main/res/xml/network_security_config.xml"), "utf8");
  expect(written).toBe(NETWORK_SECURITY_XML);
});
