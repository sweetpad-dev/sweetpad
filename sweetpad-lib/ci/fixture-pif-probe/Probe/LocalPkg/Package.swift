// swift-tools-version: 5.9
import PackageDescription
let package = Package(name: "LocalPkg", products: [.library(name: "LocalPkgLib", targets: ["LocalPkgLib"])], targets: [.target(name: "LocalPkgLib")])
