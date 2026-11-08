// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "ValueKit",
    platforms: [.iOS(.v18)],
    products: [.library(name: "ValueKit", targets: ["ValueKit"])],
    targets: [.target(name: "ValueKit", resources: [.process("package.txt")])]
)
