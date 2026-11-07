# App Intents compiler captures

The Swift files are synthetic probes owned by this repository.
`regen.py` builds both probes with Xcode through the local sweetpad binary.
It requires macOS, Xcode, and XcodeGen.

```sh
cargo build -p sweetpad-cli --bin sweetpad
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer python3 -I sweetpad-cli/fixtures/appintents/regen.py
```

The current captures use Xcode 27.2 (`27B52`).
The generator normalizes source paths to file names.
The tests normalize the generator identity and the order of resolver sets.
The language-model test compares decoded phrases and negative examples.
It ignores native phrase order and generation timestamps.

Registration covers basic intents, primitive parameters, enums, entities,
queries, widget configuration, authentication policies, and App Shortcuts.
Assistant schemas, explicit supported modes, custom parameter options,
and dependency metadata currently stop extraction with an error.
Localized shortcut phrase models and availability-specific metadata need more coverage.
These captures establish format compatibility, not device registration or Siri behavior.
