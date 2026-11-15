// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "Gen",
    products: [
        .library(name: "Greeting", targets: ["Greeting"]),
        .plugin(name: "GenPlugin", targets: ["GenPlugin"]),
    ],
    targets: [
        .executableTarget(name: "GenTool"),
        .plugin(name: "GenPlugin", capability: .buildTool(), dependencies: ["GenTool"]),
        .target(name: "Greeting", plugins: ["GenPlugin"]),
    ]
)
