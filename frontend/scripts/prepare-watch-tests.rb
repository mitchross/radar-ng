# Adds a reproducible native watchOS UI test target to Expo's generated project.
require 'xcodeproj'
root = File.expand_path('..', __dir__)
project = Xcodeproj::Project.open(File.join(root, 'ios/radarng.xcodeproj'))
watch = project.targets.find { |target| target.name == 'radar-ngWatch' }
abort 'Run Expo iOS prebuild first (Watch target missing)' unless watch
name = 'RadarWatchUITests'
target = project.targets.find { |t| t.name == name } || project.new_target(:ui_test_bundle, name, :watchos, '10.0')
target.build_configurations.each do |config|
  config.build_settings.merge!({
    'PRODUCT_BUNDLE_IDENTIFIER' => 'com.vanillax.radar-ng.watch.uitests',
    'SWIFT_VERSION' => '5.0',
    'PRODUCT_NAME' => '$(TARGET_NAME)',
    'GENERATE_INFOPLIST_FILE' => 'YES',
    'TEST_TARGET_NAME' => watch.name,
    'TARGETED_DEVICE_FAMILY' => '4',
    'CODE_SIGNING_ALLOWED' => 'NO'
  })
end
file_path = File.join(root, 'tests/watch/WatchUITests.swift')
unless target.source_build_phase.files_references.any? { |file| file.real_path.to_s == file_path }
  target.source_build_phase.add_file_reference(project.main_group.new_file(file_path))
end
target.add_dependency(watch) unless target.dependencies.any? { |d| d.target == watch }
project.save
scheme = Xcodeproj::XCScheme.new
scheme.add_build_target(watch)
scheme.add_test_target(target)
scheme.set_launch_target(watch)
scheme.save_as(project.path, 'WatchQA', true)
