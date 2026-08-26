// swift-tools-version:5.9
import PackageDescription
let package = Package(
  name: "NestedDep",
  products: [.library(name: "NestedLib", targets: ["Nested"])],
  targets: [.target(name: "Nested"), .testTarget(name: "NestedTests", dependencies: ["Nested"])]
)
