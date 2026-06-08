// swift-tools-version: 6.0
import PackageDescription
import CompilerPluginSupport

// A minimal third-party macro package: a `.macro` plugin target
// (`SweetMacroMacros`) that builds to a host executable, re-exported through a
// library product (`SweetMacro`). The app consumes the library; the macro
// implementation exists only in the plugin executable.
let package = Package(
    name: "SweetMacro",
    platforms: [.macOS(.v10_15), .iOS(.v13), .tvOS(.v13), .watchOS(.v6), .macCatalyst(.v13)],
    products: [
        .library(name: "SweetMacro", targets: ["SweetMacro"]),
    ],
    dependencies: [
        .package(url: "https://github.com/swiftlang/swift-syntax.git", from: "600.0.0"),
    ],
    targets: [
        .macro(
            name: "SweetMacroMacros",
            dependencies: [
                .product(name: "SwiftSyntaxMacros", package: "swift-syntax"),
                .product(name: "SwiftCompilerPlugin", package: "swift-syntax"),
            ]
        ),
        .target(name: "SweetMacro", dependencies: ["SweetMacroMacros"]),
    ]
)
