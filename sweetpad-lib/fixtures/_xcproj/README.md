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
