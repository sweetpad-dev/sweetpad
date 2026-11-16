# Swift Build in the SweetPad package

`libexec/sweetpad/swbuild`, `SWBBuildServiceBundle` and the `SwiftBuild_*`
resource bundles beside them are built from Swift Build
(https://github.com/swiftlang/swift-build), Copyright Apple Inc. and the Swift
project authors, under the Apache License 2.0 with the Runtime Library
Exception in `LICENSE.txt`. The licenses of the packages Swift Build depends
on are in `dependencies/`.

SweetPad modified Swift Build and its llbuild dependency. Each change is one
of the patch files in `patches/`, applied to the release tag named in
`libexec/sweetpad/sweetpad-swift-build.json`:

- `swift-build-linux.patch` makes Swift Build's Apple build tasks run on
  Linux. It notes a failed Launch Services registration instead of failing
  the build, asks the tool `SWEETPAD_PROVISIONING` names for signing
  identities and profiles, finds `derq` on the search path, defines Mach-O
  structures and text encodings without the Darwin SDK, reads LLVM libtool
  versions, computes MD5 without CryptoKit, and removes loose asset catalog
  outputs the previous compile listed.
- `swift-build-generic-debug-bridges.patch` corrects two self-nesting
  Optional Objective-C bridge helpers in generated C++ headers.
- `swift-llbuild-directory-race.patch` lets llbuild's directory iterator
  skip entries removed while it reads them.
- `swift-llbuild-spawn-descriptors.patch` closes descriptors above the
  standard three in the processes llbuild starts, as Darwin's spawn does.

`sweetpad-macros` is built from `sweetpad-cli/macros` in the SweetPad
repository with swift-syntax, whose license is in
`../sweetpad-macros/swift-syntax/`.

The `lib*.so` files beside them are the Swift runtime, Dispatch and
Foundation libraries of the Swift release named in
`sweetpad-swift-build.json`, unmodified apart from stripped symbol tables.
They are under the Swift project's license in `../swift/LICENSE.txt`.
