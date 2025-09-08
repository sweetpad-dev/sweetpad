load("@build_bazel_rules_swift//swift:swift.bzl", "swift_library")
load("//bazel_support/rules:dd_ios_application.bzl", "dd_ios_application")
load("//bazel_support/rules:match_profile.bzl", "match_profile")

# Swift Library for SecurityDemo

swift_library(
    name = "SecurityDemo.library",
    srcs = glob([
        "SecurityDemo/**/*.swift",
    ]),
    module_name = "SecurityDemo",
    visibility = ["//visibility:private"],
    deps = [
        "//Packages/AppAttestation:AppAttestation",
        "//Packages/DeviceAnalytics:DeviceAnalytics",
        "//Packages/FraudDetection:FraudDetection",
    ],
)

# Example SecurityDemo App

dd_ios_application(
    name = "SecurityDemo",
    bundle_id = "com.example.SecurityDemo",
    entitlements_template = "SecurityDemo/SecurityDemo.entitlements",
    info_plist_template = "Info.plist",
    minimum_os_version = "17.0",
    resources = glob([
        "SecurityDemo/Assets.xcassets/**",
        "SecurityDemo/Preview Content/**",
    ]),
    deps = [
        ":SecurityDemo.library",
    ],
    families = ["iphone", "ipad"],
    sdk_frameworks = [
        "DeviceCheck",
        "CoreLocation",
        "AdSupport",
        "CryptoKit",
    ],
)
