// Keep the real AndroidConfig helpers; only expose the strings mod so the plugin
// can be driven without running a prebuild.
jest.mock("@expo/config-plugins", () => {
  const actual = jest.requireActual("@expo/config-plugins");
  return { ...actual, withStringsXml: (config, mod) => ({ ...config, stringsMod: mod }) };
});

const plugin = require("../../plugins/withAndroidAppLabel");

const appName = (modResults) =>
  modResults.resources.string.find((s) => s.$.name === "app_name")?._;

test("sets the launcher label without touching expo.name", () => {
  const config = { name: "radar-ng" };
  const native = {
    modResults: { resources: { string: [{ $: { name: "app_name" }, _: "radar-ng" }] } },
  };

  const output = plugin(config).stringsMod(native);

  expect(appName(output.modResults)).toBe("Radar NG");
  // expo.name is what names ios/radarng and the Android rootProject, so it must
  // not be rewritten here.
  expect(config.name).toBe("radar-ng");
});

test("adds the label when the resource is absent", () => {
  const native = { modResults: { resources: {} } };
  const output = plugin({ name: "radar-ng" }).stringsMod(native);
  expect(appName(output.modResults)).toBe("Radar NG");
});
