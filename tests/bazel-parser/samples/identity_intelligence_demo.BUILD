load("@build_bazel_rules_swift//swift:swift.bzl", "swift_library")
load("//bazel_support/rules:dd_ios_application.bzl", "dd_ios_application")
load("//bazel_support/rules:match_profile.bzl", "match_profile")

# Swift Library for IdentityIntelligenceDemo

swift_library(
    name = "IdentityIntelligenceDemo.library",
    srcs = glob([
        "IdentityIntelligenceDemo/**/*.swift",
    ]),
    module_name = "IdentityIntelligenceDemo",
    visibility = ["//visibility:private"],
    deps = [
        "//Packages/DoordashAttestation:DoordashAttestation",
        "//Packages/DeviceIntelligence:DeviceIntelligence",
        "//Packages/BotDetector:BotDetector",
    ],
)

# DoorDash IdentityIntelligenceDemo App

dd_ios_application(
    name = "IdentityIntelligenceDemo",
    bundle_id = "com.doordash.AppAttestationDemo",
    entitlements_template = "IdentityIntelligenceDemo/IdentityIntelligenceDemo.entitlements",
    info_plist_template = "Info.plist",
    minimum_os_version = "17.0",
    resources = glob([
        "IdentityIntelligenceDemo/Assets.xcassets/**",
        "IdentityIntelligenceDemo/Preview Content/**",
    ]),
    deps = [
        ":IdentityIntelligenceDemo.library",
    ],
    families = ["iphone", "ipad"],
    sdk_frameworks = [
        "DeviceCheck",
        "CoreLocation",
        "AdSupport",
        "CryptoKit",
    ],
)
