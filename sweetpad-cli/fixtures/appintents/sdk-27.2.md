# iOS App Intents SDK coverage

Xcode 27.2 (27B52) and Xcode 26.6 (17F113) expose the same 63 checklist entries. The inventory covers public `IntentParameter` initializer labels across value types, string input settings and connection cases, protocols refining `IntentResult`, and the App Intent summary builders. Each match applies to the Swift types and constant expressions in the linked probe.

| Xcode | iOS SDK | Public declarations including overloads | Native registration |
| --- | --- | ---: | --- |
| 26.6 (17F113) | 26.5 | [542 declarations](sdk-26.5-signatures.txt) | [Seven probe fingerprints](sdk-26.5-registration.json) |
| 27.2 (27B52) | 27.2 | [587 declarations](sdk-27.2-signatures.txt) | [Seven probe fingerprints](sdk-27.2-registration.json) |

CI compares the installed SDK’s public symbol graph, including overlays, with the most recent reviewed snapshot at or below that SDK version. It checks additions, removals and changes to overloads, including declarations that reuse existing labels. The registration check also compares fresh Xcode output to Linux and to the native fingerprints in the table. [SDK check](sdk_coverage.py) · [Registration check](verify_registration.py)

## Parameter initializer options

| Declaration | Classification | Evidence |
| --- | --- | --- |
| `parameter.controlStyle` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.currencyCodes` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.default` | Match | [TypesIntent](Types.swift#L25) · [Types.actionsdata](Types.actionsdata) |
| `parameter.defaultUnit` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.defaultUnitAdjustForLocale` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.defaultValue` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.description` | Match | [FileInputIntent](Files.swift#L5) · [Files.actionsdata](Files.actionsdata) |
| `parameter.displayName` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.displayStyle` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.inclusiveRange` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) · [RangeProbe](Unsupported.swift#L30) |
| `parameter.inputConnectionBehavior` | Match | [ConnectionProbe](ResultProtocols.swift#L69) · [Xcode registration](ResultProtocols.actionsdata#L164) |
| `parameter.inputOptions` | Match | [allOff](InputOptionsProbe.swift#L9) · [Xcode parameter](InputOptionsProbe.actions.json#L29) |
| `parameter.kind` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.mode` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.optionsProvider` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) · [OptionsProviderProbe](Unsupported.swift#L40) |
| `parameter.query` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.requestDisambiguationDialog` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.requestValueDialog` | No effect | [PromptProbe](ResultProtocols.swift#L10) · [Xcode registration](ResultProtocols.actionsdata#L550) · [native control comparison](verify_registration.py#L61) |
| `parameter.resolvers` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.size` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.supportedContentTypes` | Match | [FileInputIntent](Files.swift#L5) · [Files.actionsdata](Files.actionsdata) |
| `parameter.supportedTypeIdentifiers` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.supportedValues` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.supportsNegativeNumbers` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.title` | Match | [TypesIntent](Types.swift#L25) · [Types.actionsdata](Types.actionsdata) |
| `parameter.unit` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |
| `parameter.unitAdjustForLocale` | Diagnostic | [per-option extractor diagnostic](verify_registration.py#L98) |

## Connection behavior

| Declaration | Classification | Evidence |
| --- | --- | --- |
| `connection.connectToPreviousIntentResult` | Match | [ConnectionProbe](ResultProtocols.swift#L69) · [Xcode registration](ResultProtocols.actionsdata#L164) |
| `connection.default` | Match | [ConnectionProbe](ResultProtocols.swift#L69) · [Xcode registration](ResultProtocols.actionsdata#L164) |
| `connection.never` | Match | [ConnectionProbe](ResultProtocols.swift#L69) · [Xcode registration](ResultProtocols.actionsdata#L164) |

## String input options

| Declaration | Classification | Evidence |
| --- | --- | --- |
| `inputOptions.CapitalizationType.allCharacters` | Match | [allOn](InputOptionsProbe.swift#L11) · [Xcode parameter](InputOptionsProbe.actions.json#L125) |
| `inputOptions.CapitalizationType.none` | Match | [allOff](InputOptionsProbe.swift#L9) · [Xcode parameter](InputOptionsProbe.actions.json#L29) |
| `inputOptions.CapitalizationType.sentences` | Match | [url](InputOptionsProbe.swift#L17) · [Xcode parameter](InputOptionsProbe.actions.json#L352) |
| `inputOptions.CapitalizationType.words` | Match | [punct](InputOptionsProbe.swift#L15) · [Xcode parameter](InputOptionsProbe.actions.json#L280) |
| `inputOptions.KeyboardType.URL` | Match | [url](InputOptionsProbe.swift#L17) · [Xcode parameter](InputOptionsProbe.actions.json#L352) |
| `inputOptions.KeyboardType.asciiCapable` | Match | [allOn](InputOptionsProbe.swift#L11) · [Xcode parameter](InputOptionsProbe.actions.json#L125) |
| `inputOptions.KeyboardType.default` | Match | [allOff](InputOptionsProbe.swift#L9) · [Xcode parameter](InputOptionsProbe.actions.json#L29) |
| `inputOptions.KeyboardType.numberPad` | Match | [numberPad](InputOptionsProbe.swift#L19) · [Xcode parameter](InputOptionsProbe.actions.json#L424) |
| `inputOptions.KeyboardType.numbersAndPunctuation` | Match | [punct](InputOptionsProbe.swift#L15) · [Xcode parameter](InputOptionsProbe.actions.json#L280) |
| `inputOptions.autocorrect` | Match | [autocorrect](InputOptionsProbe.swift#L25) · [Xcode parameter](InputOptionsProbe.actions.json#L622) |
| `inputOptions.capitalizationType` | Match | [allOn](InputOptionsProbe.swift#L11) · [Xcode parameter](InputOptionsProbe.actions.json#L125) |
| `inputOptions.keyboardType` | Match | [allOff](InputOptionsProbe.swift#L9) · [Xcode parameter](InputOptionsProbe.actions.json#L29) |
| `inputOptions.multiline` | Match | [allOn](InputOptionsProbe.swift#L11) · [Xcode parameter](InputOptionsProbe.actions.json#L125) |
| `inputOptions.smartDashes` | Match | [dashes](InputOptionsProbe.swift#L23) · [Xcode parameter](InputOptionsProbe.actions.json#L556) |
| `inputOptions.smartQuotes` | Match | [quotes](InputOptionsProbe.swift#L21) · [Xcode parameter](InputOptionsProbe.actions.json#L490) |

## Result protocols and compositions

| Declaration | Classification | Evidence |
| --- | --- | --- |
| `result.IntentResult` | Match | [ConfirmProbe](ResultProtocols.swift#L4) · [Xcode registration](ResultProtocols.actionsdata#L3) |
| `result.OpensIntent` | Match | [OpensProbeIntent](OutputProbe.swift#L22) · [Xcode action](OutputProbe.actions.json#L182) |
| `result.OpensIntent+ProvidesDialog` | Match | [OpensWithDialogProbe](ResultProtocols.swift#L62) · [Xcode registration](ResultProtocols.actionsdata#L476) |
| `result.OpensIntent+ShowsSnippetView` | Match | [OpensWithViewProbe](ResultProtocols.swift#L54) · [Xcode registration](ResultProtocols.actionsdata#L513) |
| `result.ProvidesDialog` | Match | [DialogOnlyProbeIntent](OutputProbe.swift#L5) · [Xcode action](OutputProbe.actions.json#L46) |
| `result.ReturnsValue` | Match | [Greet](DataApp.swift#L3) · [extract.actionsdata](extract.actionsdata) |
| `result.ShowsSnippetIntent` | Match | [InteractiveResultProbe](ResultProtocols.swift#L47) · [Xcode registration](ResultProtocols.actionsdata#L436) · [ConfirmationSnippet](ResultProtocols.swift#L18) |
| `result.ShowsSnippetView` | Match | [SnippetOnlyProbeIntent](OutputProbe.swift#L10) · [Xcode action](OutputProbe.actions.json#L219) |

## Parameter summaries

| Declaration | Classification | Evidence |
| --- | --- | --- |
| `summary.Case` | Diagnostic | [SwitchProbe](Unsupported.swift#L12) · [extractor diagnostic](verify_registration.py#L70) |
| `summary.DefaultCase` | Diagnostic | [SwitchProbe](Unsupported.swift#L12) · [extractor diagnostic](verify_registration.py#L70) |
| `summary.Summary` | Match | [SummaryTwoProbeIntent](OutputProbe.swift#L35) · [Xcode action](OutputProbe.actions.json#L617) |
| `summary.Summary()` | Match | [NoSummaryTextProbeIntent](OutputProbe.swift#L60) · [Xcode action](OutputProbe.actions.json#L131) |
| `summary.Summary(_:)` | Match | [SummaryParametersOnlyProbeIntent](OutputProbe.swift#L74) · [Xcode action](OutputProbe.actions.json#L458) |
| `summary.Summary(_:table:)` | Match | [EmptySummaryTextProbeIntent](OutputProbe.swift#L68) · [Xcode action](OutputProbe.actions.json#L83) |
| `summary.Summary(_:table:_:)` | Match | [SummaryTwoProbeIntent](OutputProbe.swift#L35) · [Xcode action](OutputProbe.actions.json#L617) |
| `summary.Switch` | Diagnostic | [SwitchProbe](Unsupported.swift#L12) · [extractor diagnostic](verify_registration.py#L70) |
| `summary.When` | Diagnostic | [WhenProbe](Unsupported.swift#L3) · [extractor diagnostic](verify_registration.py#L70) |
| `summary.table` | Diagnostic | [TableProbe](Unsupported.swift#L24) · [extractor diagnostic](verify_registration.py#L70) |

An explicit non-nil localization table is covered by `summary.table`. The two plain summary initializers accepting a table match with its default nil value. The conditional probe exercises both branches of `When` and includes `Case` and `DefaultCase` in a single `Switch`; the extractor rejects the enclosing condition.

For the 20 unsupported parameter labels, the test adds each label to a valid compiler capture and requires a named error at the public extractor entry point. Range and dynamic options also have native Swift probes. Supported type identifiers for file contents and the constant input options are limited to the values in their probes. Siri presentation, runtime prompts and interaction with a rendered snippet are outside this registration comparison.
