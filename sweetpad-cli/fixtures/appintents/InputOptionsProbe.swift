// Every String.IntentInputOptions argument, explicit and defaulted.
import AppIntents

struct InputOptionsProbeIntent: AppIntent {
  static let title: LocalizedStringResource = "Input options probe"
  static let openAppWhenRun: Bool = false

  @Parameter(title: "All off", inputOptions: String.IntentInputOptions(keyboardType: .default, capitalizationType: .none, multiline: false, autocorrect: false, smartQuotes: false, smartDashes: false))
  var allOff: String?
  @Parameter(title: "All on", inputOptions: String.IntentInputOptions(keyboardType: .asciiCapable, capitalizationType: .allCharacters, multiline: true, autocorrect: true, smartQuotes: true, smartDashes: true))
  var allOn: String?
  @Parameter(title: "Empty", inputOptions: String.IntentInputOptions())
  var empty: String?
  @Parameter(title: "Punct", inputOptions: String.IntentInputOptions(keyboardType: .numbersAndPunctuation, capitalizationType: .words))
  var punct: String?
  @Parameter(title: "URL", inputOptions: String.IntentInputOptions(keyboardType: .URL, capitalizationType: .sentences))
  var url: String?
  @Parameter(title: "Number pad", inputOptions: String.IntentInputOptions(keyboardType: .numberPad))
  var numberPad: String?
  @Parameter(title: "Smart quotes only", inputOptions: String.IntentInputOptions(smartQuotes: false))
  var quotes: String?
  @Parameter(title: "Smart dashes only", inputOptions: String.IntentInputOptions(smartDashes: false))
  var dashes: String?
  @Parameter(title: "Autocorrect only", inputOptions: String.IntentInputOptions(autocorrect: false))
  var autocorrect: String?
  @Parameter(title: "With default", default: "hi", inputOptions: String.IntentInputOptions(multiline: true))
  var withDefault: String
  @Parameter(title: "Options and dialog", description: "Described", inputOptions: String.IntentInputOptions(keyboardType: .numberPad, autocorrect: false), requestValueDialog: "Which?")
  var dialog: String

  @Parameter(title: "Explicit defaults", inputOptions: String.IntentInputOptions(keyboardType: .asciiCapable, capitalizationType: .sentences, multiline: false, autocorrect: true, smartQuotes: true, smartDashes: true))
  var explicitDefaults: String?

  @MainActor
  func perform() async throws -> some IntentResult { .result() }
}
