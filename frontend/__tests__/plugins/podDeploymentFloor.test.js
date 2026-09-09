const { execFileSync } = require("node:child_process");
const { patchPodfile } = require("../../plugins/withXcode27PodDeploymentFloor");

test.each([
  "platform :ios, '26.0'",
  "ios_deployment_target = '26.0'\nplatform :ios, ios_deployment_target",
])("generated Podfile executes the floor hook: %s", (platform) => {
  const template = `${platform}
post_install do |installer|
    react_native_post_install(
      installer
    )
end
`;
  const patched = patchPodfile(template);
  expect(patchPodfile(patched)).toBe(patched);
  const harness = `
require 'ostruct'
require 'rubygems'
def platform(*); end
def react_native_post_install(*); end
def post_install
  configs = ['14.0', '16.4', '26.0', '$(inherited)'].map do |v|
    OpenStruct.new(build_settings: {'IPHONEOS_DEPLOYMENT_TARGET' => v})
  end
  target = OpenStruct.new(build_configurations: configs)
  yield OpenStruct.new(pods_project: OpenStruct.new(targets: [target]))
  actual = configs.map { |c| c.build_settings['IPHONEOS_DEPLOYMENT_TARGET'] }
  abort actual.inspect unless actual == ['15.0', '16.4', '26.0', '$(inherited)']
end
`;
  expect(() => execFileSync("ruby", ["-e", harness + patched])).not.toThrow();
});
