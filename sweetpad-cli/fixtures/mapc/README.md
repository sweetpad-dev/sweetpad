# Core Data mapping compiler capture

`Users.xcmappingmodel` comes from the Tuist Core Data fixture at commit
`a5c5bbf867ba2dfc9d2ca5ec198e9ddbf1b0e963`.
The source is `examples/xcode/generated_ios_app_with_coredata` in
[tuist/tuist](https://github.com/tuist/tuist).
Tuist licenses this fixture under the MIT license in `LICENSE.md`.

`Users.cdm` is the native Xcode 27 reference archive.
Regenerate it on macOS with your own Xcode:

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer python3 -I sweetpad-cli/fixtures/mapc/regen.py
```

The golden test compares the decoded keyed archive.
The compiler supports transform mappings and direct source-property expressions.
Custom expressions, inheritance, uniqueness constraints, and unsupported mapping types fail explicitly.

`Relationships.xcmappingmodel` adapts Wikipedia iOS's cache mapping fixture at
`df61688dab26991111e4578248413a647e5891a8`. Entity, property, and policy names
are changed to fixture names. Its MIT notice is in `LICENSE-Wikipedia.txt`.
The native capture covers relationship snapshot hashes, migration-manager
expressions, null expressions for new properties, custom policies, and copy
mapping classification. Run `regen.py` with the selected Xcode to refresh both
captures.
