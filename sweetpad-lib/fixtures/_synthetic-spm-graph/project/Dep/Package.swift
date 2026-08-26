// swift-tools-version:5.9
import PackageDescription
let package = Package(
  name: "Dep",
  products: [.library(name: "Dep", targets: ["Dep"])],
  dependencies: [.package(path: "../../DepChild")],
  targets: [
    .target(name: "Dep", dependencies: [.product(name: "DepChildLib", package: "DepChild")]),
    .testTarget(name: "DepTests", dependencies: ["Dep"]),
  ]
)
