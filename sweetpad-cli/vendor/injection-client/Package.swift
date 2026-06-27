// swift-tools-version: 5.9
//
// A thin wrapper that re-exposes the upstream InjectionNext client as a loadable
// dynamic library, with no fork and no source patches (CLI_DESIGN §9d).
//
// InjectionNext's *Xcode* `InjectionBundle` target links XCTest + Quick + Nimble
// for its test-reload feature; those are the only Xcode-versioned dependencies in
// the client. Its *SPM* product carries none of them (and SPM defines
// SWIFT_PACKAGE, which the engine's `canImport(Nimble)` build sentinel keys on),
// so building the SPM product yields an XCTest-free client that depends only on
// ABI-stable OS/runtime libraries — hence one prebuilt is portable across Xcode
// versions and can be bundled into the `sweetpad` binary.
//
// We only add a `.dynamic` product (upstream ships static ones) so the result is
// loadable via DYLD_INSERT_LIBRARIES. Run `./build.sh` to (re)produce the
// vendored dylib; bump `revision` to track upstream.
import PackageDescription

let package = Package(
    name: "SweetpadInjectionClient",
    platforms: [.iOS(.v13)],
    products: [
        .library(name: "SweetpadInjectionClient", type: .dynamic, targets: ["SweetpadInjectionClient"]),
    ],
    dependencies: [
        // InjectionNext 2.0.1RC8. Pinned by commit so the build is reproducible.
        .package(url: "https://github.com/johnno1962/InjectionNext",
                 revision: "843e52fcb433c671d2074c65e78b7048cf1b7920"),
    ],
    targets: [
        .target(
            name: "SweetpadInjectionClient",
            dependencies: [.product(name: "InjectionNext", package: "InjectionNext")],
            // -all_load keeps the client's ObjC `+load` (its connect-on-launch
            // hook) and the engine objects, which the linker would otherwise
            // dead-strip from a dynamic library that only *depends on* the static
            // InjectionNext product without referencing its symbols directly.
            linkerSettings: [.unsafeFlags(["-Xlinker", "-all_load"])]
        ),
    ]
)
