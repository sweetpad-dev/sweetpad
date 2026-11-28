import AppIntents
import SwiftUI

struct ConfirmProbe: AppIntent {
    static let title: LocalizedStringResource = "Confirm"
    @Parameter(title: "Text") var text: String?
    func perform() async throws -> some IntentResult { .result() }
}

struct PromptProbe: AppIntent {
    // Keep the two actions identical except for type identity and the prompt.
    static let title: LocalizedStringResource = "Confirm"
    @Parameter(title: "Text", requestValueDialog: "Which text?") var text: String?
    func perform() async throws -> some IntentResult { .result() }
}

@available(iOS 26.0, macOS 26.0, tvOS 26.0, watchOS 26.0, visionOS 26.0, macCatalyst 26.0, *)
struct ConfirmationSnippet: SnippetIntent {
    static let title: LocalizedStringResource = "Confirmation snippet"
    @MainActor
    func perform() async throws -> some IntentResult & ShowsSnippetView {
        .result(view: Button("Confirm", intent: ConfirmProbe()))
    }
}

@available(iOS, introduced: 26.0, deprecated: 27.0, obsoleted: 30.0)
@available(tvOS, unavailable)
struct DiscoverableSnippet: SnippetIntent {
    static let title: LocalizedStringResource = "Discoverable snippet"
    static let isDiscoverable = true
    @MainActor
    func perform() async throws -> some IntentResult & ShowsSnippetView {
        .result(view: Text("Visible"))
    }
}

@available(iOS 26.0, *)
struct ValueSnippet: SnippetIntent {
    static let title: LocalizedStringResource = "Snippet with a value"
    @MainActor
    func perform() async throws -> some IntentResult & ReturnsValue<String> & ShowsSnippetView {
        .result(value: "Confirmed", view: Text("Confirmed"))
    }
}

@available(iOS 26.0, *)
struct InteractiveResultProbe: AppIntent {
    static let title: LocalizedStringResource = "Show confirmation"
    func perform() async throws -> some IntentResult & ShowsSnippetIntent {
        .result(snippetIntent: ConfirmationSnippet())
    }
}

struct OpensWithViewProbe: AppIntent {
    static let title: LocalizedStringResource = "Open with view"
    @MainActor
    func perform() async throws -> some IntentResult & OpensIntent & ShowsSnippetView {
        .result(opensIntent: ConfirmProbe(), view: Text("Opening"))
    }
}

struct OpensWithDialogProbe: AppIntent {
    static let title: LocalizedStringResource = "Open with dialog"
    func perform() async throws -> some IntentResult & OpensIntent & ProvidesDialog {
        .result(opensIntent: ConfirmProbe(), dialog: "Opening")
    }
}

struct ConnectionProbe: AppIntent {
    static let title: LocalizedStringResource = "Connection behavior"
    @Parameter(title: "Default", inputConnectionBehavior: .default) var automatic: String?
    @Parameter(title: "Never", inputConnectionBehavior: .never) var never: String?
    @Parameter(title: "Previous", inputConnectionBehavior: .connectToPreviousIntentResult) var previous: String?
    func perform() async throws -> some IntentResult { .result() }
}
