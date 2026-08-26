// swift-tools-version:5.9
import PackageDescription
let package = Package(
  name: "MultiLib",
  products: [
    .library(name: "LibA", targets: ["TA"]),
    .executable(name: "ExecB", targets: ["TB"]),
  ],
  dependencies: [.package(path: "../NestedDep")],
  targets: [
    .target(name: "TA", dependencies: [.product(name: "NestedLib", package: "NestedDep")]),
    .executableTarget(name: "TB"),
    .target(name: "TC"),
    .testTarget(name: "TATests", dependencies: ["TA"]),
  ]
)
