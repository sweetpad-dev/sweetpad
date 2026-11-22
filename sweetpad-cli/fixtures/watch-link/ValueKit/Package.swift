// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "ValueKit",
    platforms: [.watchOS(.v11)],
    products: [.library(name: "ValueKit", type: .dynamic, targets: ["ValueKit"])],
    targets: [.target(name: "ValueKit")]
)
