# CocoaPods 1.16.2 bundles xcodeproj 1.27.0, which can read Xcode project
# objectVersion 77 but is missing the compatibility string for 70.
require 'xcodeproj/constants'

compatibility_versions = Xcodeproj::Constants::COMPATIBILITY_VERSION_BY_OBJECT_VERSION

unless compatibility_versions.key?(70)
  patched_versions = compatibility_versions.merge(70 => 'Xcode 16.0').freeze

  Xcodeproj::Constants.send(:remove_const, :COMPATIBILITY_VERSION_BY_OBJECT_VERSION)
  Xcodeproj::Constants.const_set(:COMPATIBILITY_VERSION_BY_OBJECT_VERSION, patched_versions)
end
