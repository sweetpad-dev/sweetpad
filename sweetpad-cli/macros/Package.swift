// swift-tools-version: 6.0

// Stand-ins for the macro plugins Apple ships only as macOS libraries in
// Xcode, for building on Linux. The compiler finds them through the
// `-load-plugin-executable` option `sweetpad build` adds; each library
// target is named after Apple's plugin module, so the macro types resolve
// as the SDK's `#externalMacro` declarations name them.
//
// `FoundationMacros` is swift-foundation's own source, taken unchanged from
// the swift-6.4.0-RELEASE tag (Apache 2.0, `LICENSE.md` beside it). The
// toolchain's copy is built for swift-corelibs-foundation and spells its
// expansions with `FoundationEssentials`; built with `FOUNDATION_FRAMEWORK`
// they name `Foundation`, which is what the SDK's Foundation declares.

import PackageDescription

let package = Package(
    name: "sweetpad-macros",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "sweetpad-macros", targets: ["sweetpad-macros"])
    ],
    dependencies: [
        .package(url: "https://github.com/swiftlang/swift-syntax.git", "601.0.0"..<"700.0.0")
    ],
    targets: [
        .target(
            name: "SwiftDataMacros",
            dependencies: [
                .product(name: "SwiftSyntax", package: "swift-syntax"),
                .product(name: "SwiftSyntaxBuilder", package: "swift-syntax"),
                .product(name: "SwiftSyntaxMacros", package: "swift-syntax"),
            ]
        ),
        .target(
            name: "FoundationMacros",
            dependencies: [
                .product(name: "SwiftDiagnostics", package: "swift-syntax"),
                .product(name: "SwiftIfConfig", package: "swift-syntax"),
                .product(name: "SwiftSyntax", package: "swift-syntax"),
                .product(name: "SwiftSyntaxBuilder", package: "swift-syntax"),
                .product(name: "SwiftSyntaxMacros", package: "swift-syntax"),
            ],
            exclude: ["LICENSE.md"],
            swiftSettings: [.define("FOUNDATION_FRAMEWORK")]
        ),
        .target(
            name: "PreviewsMacros",
            dependencies: [
                .product(name: "SwiftDiagnostics", package: "swift-syntax"),
                .product(name: "SwiftSyntax", package: "swift-syntax"),
                .product(name: "SwiftSyntaxBuilder", package: "swift-syntax"),
                .product(name: "SwiftSyntaxMacros", package: "swift-syntax"),
            ]
        ),
        .executableTarget(
            name: "sweetpad-macros",
            dependencies: [
                "FoundationMacros",
                "PreviewsMacros",
                "SwiftDataMacros",
                .product(name: "SwiftCompilerPlugin", package: "swift-syntax"),
            ]
        ),
    ]
)
