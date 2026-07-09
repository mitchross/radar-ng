const { withPodfile } = require("@expo/config-plugins");

const POD_FLOOR_DECLARATION = "minimum_pod_deployment_target = '15.0'";

const POD_FLOOR_BLOCK = `    # Xcode 27 errors on pod targets below iOS 15. Keep the app target at iOS 26,
    # but only lift third-party pod targets that still declare an older floor.
    minimum_pod_version = Gem::Version.new(minimum_pod_deployment_target)
    installer.pods_project.targets.each do |pod_target|
      pod_target.build_configurations.each do |build_configuration|
        deployment_target = build_configuration.build_settings['IPHONEOS_DEPLOYMENT_TARGET']
        next unless deployment_target

        begin
          next unless Gem::Version.new(deployment_target) < minimum_pod_version
        rescue ArgumentError
          next
        end

        build_configuration.build_settings['IPHONEOS_DEPLOYMENT_TARGET'] = minimum_pod_deployment_target
      end
    end`;

function patchPodfile(contents) {
  let next = contents;

  if (!next.includes(POD_FLOOR_DECLARATION)) {
    next = next.replace(
      /^(ios_deployment_target = .+)$/m,
      `$1\n${POD_FLOOR_DECLARATION}`,
    );
  }

  if (!next.includes("minimum_pod_version = Gem::Version.new(minimum_pod_deployment_target)")) {
    next = next.replace(
      /(    react_native_post_install\([\s\S]*?^    \)\n)/m,
      `$1\n${POD_FLOOR_BLOCK}\n`,
    );
  }

  return next;
}

module.exports = (config) => {
  return withPodfile(config, (c) => {
    c.modResults.contents = patchPodfile(c.modResults.contents);
    return c;
  });
};

module.exports.patchPodfile = patchPodfile;
