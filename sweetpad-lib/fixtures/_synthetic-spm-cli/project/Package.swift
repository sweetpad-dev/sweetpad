// swift-tools-version:5.9
import PackageDescription

// A deliberately small but representative package for the SwiftPM CLI oracle
// (tests/spm_oracle.rs): one library product, one executable product, and a
// test target. xcodebuild synthesizes a scheme per product; sweetpad derives
// the same set from `swift package dump-package` without xcodebuild.
let package = Package(
    name: "SweetpadSpmDemo",
    products: [
        .library(name: "DemoKit", targets: ["DemoKit"]),
        .executable(name: "demo", targets: ["demo"]),
    ],
    targets: [
        .target(name: "DemoKit"),
        .executableTarget(name: "demo", dependencies: ["DemoKit"]),
        .testTarget(name: "DemoKitTests", dependencies: ["DemoKit"]),
    ]
)
