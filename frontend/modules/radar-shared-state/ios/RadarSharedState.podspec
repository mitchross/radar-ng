require 'json'

package = JSON.parse(File.read(File.join(__dir__, '..', 'package.json')))

Pod::Spec.new do |s|
  s.name           = 'RadarSharedState'
  s.version        = package['version']
  s.summary        = package['description']
  s.description    = package['description']
  s.license        = package['license']
  s.author         = 'Radar NG'
  s.homepage       = 'https://github.com/mitchross/radar-ng'
  s.platforms      = { :ios => '26.0' }
  s.source         = { git: 'https://github.com/mitchross/radar-ng.git' }
  s.dependency 'ExpoModulesCore'
  s.static_framework = true
  s.source_files = "**/*.{h,m,swift}"
end
