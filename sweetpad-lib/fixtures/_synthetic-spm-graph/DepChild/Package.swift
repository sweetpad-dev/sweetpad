// swift-tools-version:5.9
import PackageDescription
let package = Package(
  name: "DepChild",
  products: [.library(name: "DepChildLib", targets: ["DC"])],
  targets: [.target(name: "DC"), .testTarget(name: "DCTests", dependencies: ["DC"])]
)
