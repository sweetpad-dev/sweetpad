// swift-tools-version:5.9
// A tiny Swift package (executable + test) used by the cli-smoke job to exercise
// the CLI's SPM paths: scheme list, build, test, and `app run` (swift run).
import PackageDescription

let package = Package(
    name: "SweetpadCITool",
    targets: [
        .executableTarget(name: "SweetpadCITool"),
        .testTarget(name: "SweetpadCIToolTests", dependencies: ["SweetpadCITool"]),
    ]
)
