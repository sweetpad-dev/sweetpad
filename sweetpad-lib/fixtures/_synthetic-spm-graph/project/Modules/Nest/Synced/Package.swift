// swift-tools-version:5.9
import PackageDescription
let package = Package(
  name: "Synced",
  products: [.library(name: "SyncedLib", targets: ["SA"])],
  targets: [.target(name: "SA")]
)
