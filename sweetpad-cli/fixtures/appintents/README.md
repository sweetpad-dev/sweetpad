# App Intents compiler captures

The Swift files are synthetic probes owned by this repository.
`regen.py` builds the probes with Xcode through the local sweetpad binary.
It requires macOS, Xcode, and XcodeGen.

```sh
cargo build -p sweetpad-cli --bin sweetpad
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer python3 -I sweetpad-cli/fixtures/appintents/regen.py
```

The current captures use Xcode 27.2 (`27B52`).
The generator normalizes source paths to file names.
The tests normalize the generator identity, the order of resolver sets,
and the order of `typeSpecificMetadata` pairs, which Xcode writes in hash order.
The language-model test compares decoded phrases and negative examples.
It ignores native phrase order and generation timestamps.

Registration covers basic intents, primitive parameters, enums, entities,
queries, widget configuration, authentication policies, App Shortcuts,
`String.IntentInputOptions`, output flags, and plain parameter summaries.
The input options probe covers scalar strings, arrays, and sets. The summary
probes cover omitted text, an explicit empty string, interpolation, and a
trailing list of parameters. The `*.actions.json` files hold the probe actions.
Assistant schemas, explicit supported modes, other custom parameter options,
conditional summaries, summaries with a localization table, and dependency
metadata currently stop extraction with an error.
Localized shortcut phrase models and availability-specific metadata need more coverage.
These captures establish format compatibility, not device registration or Siri behavior.

## SDK upgrade checks

The iOS 27.2 checklist links each parameter option, result protocol, and summary
form to its evidence. It also lists the two result compositions involving
`OpensIntent`. The inventory records the public overloads within each family.
[SDK coverage and remaining gaps](sdk-27.2.md)

On a Mac with XcodeGen, run the following from the repository root. Use an empty
capture directory for each Xcode version. Set `DEVELOPER_DIR` to select Xcode.

```sh
cargo build --locked -p sweetpad-cli --bin sweetpad
python3 -B -I sweetpad-cli/fixtures/appintents/sdk_coverage.py
python3 -B -I sweetpad-cli/fixtures/appintents/regen.py --export-registration .build/intent-captures
```

Copy the capture directory to Linux, build the CLI with the same Cargo command,
and run the replay. CI transfers the synthetic compiler inputs and registrations
between its macOS and Linux jobs.

```sh
python3 -B -I sweetpad-cli/fixtures/appintents/verify_registration.py .build/intent-captures
```

The replay compares every field in `extract.actionsdata` and `version.json`
for each supported probe. Generator identity and the order of resolver sets
and metadata pairs may differ. It also checks native controls and the named
diagnostics for unsupported declarations. The baseline and newer SDKs get an
additional comparison with the checked-in Xcode registrations. For a new SDK,
version the checklist and its signature snapshot, review changed overloads, add
probe evidence, then run `sdk_coverage.py --update-signatures` and regenerate
the affected fixtures. The workflow performs read-only checks on PRs and weekly.
