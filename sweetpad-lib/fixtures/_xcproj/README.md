# `project.xcproj` fixtures

Xcode-authored documents in the JSON project format, for the byte-exact half of
`tests/serializer_roundtrip.rs`. Produced by converting the corresponding
`.xcodeproj` from `corpus/`:

    DEVELOPER_DIR=/Applications/Xcode-27.2.0-Beta.app/Contents/Developer \
      xcodebuild -project <project>.xcodeproj -convert-project "Xcode Project"

Xcode 27.2 is the first release that writes the format; 27.0 and 27.1 read it
but cannot produce it. Xcode's output and `xcrun xcprojformatter`'s agree
byte for byte, on these two and on the other 59 projects in the corpus, so
either can regenerate them.

`SweetpadCIApp` is small enough to read end to end. `NetNewsWire` is the widest
spread the corpus has: 83 `}, {` seams, 104 compact objects, synchronized
folders with exception sets, and a package reference.

`PackageProbe` and `SpmStaticLibrary` are pairs for `tests/spm_xcproj.rs`. Each
is Xcode 27.2's conversion of one pbxproj project before `sweetpad dependency
add` edited it, and the `Linked` document is the conversion after. `PackageProbe`
is a `sweetpad project new --platform macos` app given a remote package for
each requirement kind and one local package; `SpmStaticLibrary` is
`fixtures/_synthetic-spm`'s static library given one remote package, which a
static library takes as a target dependency. The diff between each pair holds
nothing but the package changes.
