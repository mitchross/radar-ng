#!/usr/bin/env ruby
require 'xcodeproj'
require 'fileutils'
require 'json'
root = File.expand_path('..', __dir__)
output = File.expand_path(ENV.fetch('CARPLAY_REVIEW_OUTPUT', '/tmp/radar-carplay-review-22361864'))
FileUtils.mkdir_p(output)
project_path = File.join(output, 'CarPlayReview.xcodeproj')
project = Xcodeproj::Project.new(project_path)
target = project.new_target(:application, 'CarPlayReview', :ios, '26.0')
sources = %w[RadarAPI RadarRouteProgress RadarLocationManager RadarDrivingSession RadarTileOverlay RadarManeuverFactory RadarMapController]
sources.each { |name| target.source_build_phase.add_file_reference(project.main_group.new_file(File.join(root, 'targets/carplay', "#{name}.swift"))) }
target.source_build_phase.add_file_reference(project.main_group.new_file(File.join(root, 'tests/carplay/CarPlayReviewHost.swift')))
team = JSON.parse(File.read(File.join(root, 'app.json')))['expo']['ios']['appleTeamId']
target.build_configurations.each do |configuration|
  configuration.build_settings.merge!({
    'PRODUCT_BUNDLE_IDENTIFIER' => 'com.vanillax.radar-ng.carplay-review',
    'PRODUCT_NAME' => 'CarPlayReview',
    'DEVELOPMENT_TEAM' => team,
    'CODE_SIGN_STYLE' => 'Automatic',
    'SWIFT_VERSION' => '5.0',
    'GENERATE_INFOPLIST_FILE' => 'YES',
    'INFOPLIST_KEY_CFBundleDisplayName' => 'Radar Nav Review',
    'INFOPLIST_KEY_UILaunchScreen_Generation' => 'YES',
    'INFOPLIST_KEY_UIApplicationSceneManifest_Generation' => 'YES',
    'INFOPLIST_KEY_UISupportedInterfaceOrientations' => 'UIInterfaceOrientationPortrait',
    'INFOPLIST_KEY_NSLocationWhenInUseUsageDescription' => 'The review uses replayed route positions; real location is not required.',
    'TARGETED_DEVICE_FAMILY' => '1',
    'MARKETING_VERSION' => '1.0',
    'CURRENT_PROJECT_VERSION' => '1',
  })
end
project.save
scheme = Xcodeproj::XCScheme.new
scheme.add_build_target(target)
scheme.set_launch_target(target)
scheme.save_as(project_path, 'CarPlayReview', true)
puts project_path
