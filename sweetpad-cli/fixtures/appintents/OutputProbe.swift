// Output flags and parameter summaries.
import AppIntents
import SwiftUI

struct DialogOnlyProbeIntent: AppIntent {
  static let title: LocalizedStringResource = "Dialog only"
  func perform() async throws -> some IntentResult & ProvidesDialog { .result(dialog: "Done") }
}

struct SnippetOnlyProbeIntent: AppIntent {
  static let title: LocalizedStringResource = "Snippet only"
  @MainActor
  func perform() async throws -> some IntentResult & ShowsSnippetView { .result(view: Text("Done")) }
}

struct ValueSnippetProbeIntent: AppIntent {
  static let title: LocalizedStringResource = "Value and snippet"
  @MainActor
  func perform() async throws -> some IntentResult & ReturnsValue<String> & ShowsSnippetView { .result(value: "v", view: Text("Done")) }
}

struct OpensProbeIntent: AppIntent {
  static let title: LocalizedStringResource = "Opens"
  func perform() async throws -> some IntentResult & OpensIntent { .result(opensIntent: DialogOnlyProbeIntent()) }
}

struct AllOutputsProbeIntent: AppIntent {
  static let title: LocalizedStringResource = "All outputs"
  @MainActor
  func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog & ShowsSnippetView & OpensIntent {
    .result(value: "v", opensIntent: DialogOnlyProbeIntent(), dialog: "Done", view: Text("Done"))
  }
}

struct SummaryTwoProbeIntent: AppIntent {
  static let title: LocalizedStringResource = "Summary two"
  @Parameter(title: "First") var first: String
  @Parameter(title: "Second") var second: Int
  @Parameter(title: "Third", default: false) var third: Bool
  static var parameterSummary: some ParameterSummary {
    Summary("Send \(\.$second) to \(\.$first), then \(\.$second) again") { \.$third }
  }
  func perform() async throws -> some IntentResult { .result() }
}

struct SummaryBuilderProbeIntent: AppIntent {
  static let title: LocalizedStringResource = "Summary builder"
  @Parameter(title: "First") var first: String
  @Parameter(title: "Second") var second: Int
  @Parameter(title: "Third", default: false) var third: Bool
  static var parameterSummary: some ParameterSummary {
    Summary("Just text") {
      \.$third
      \.$first
    }
  }
  func perform() async throws -> some IntentResult { .result() }
}

struct NoSummaryTextProbeIntent: AppIntent {
  static let title: LocalizedStringResource = "No summary text"
  static var parameterSummary: some ParameterSummary { Summary() }
  func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
    .result(value: "ok", dialog: "Done")
  }
}

struct EmptySummaryTextProbeIntent: AppIntent {
  static let title: LocalizedStringResource = "Empty summary text"
  static var parameterSummary: some ParameterSummary { Summary("") }
  func perform() async throws -> some IntentResult { .result() }
}

struct SummaryParametersOnlyProbeIntent: AppIntent {
  static let title: LocalizedStringResource = "Summary parameters only"
  @Parameter(title: "First") var first: String
  @Parameter(title: "Second") var second: Int
  static var parameterSummary: some ParameterSummary {
    Summary {
      \.$second
      \.$first
    }
  }
  func perform() async throws -> some IntentResult { .result() }
}
