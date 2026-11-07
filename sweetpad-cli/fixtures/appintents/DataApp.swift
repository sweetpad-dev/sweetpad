import AppIntents

struct Greet: AppIntent {
    static let title: LocalizedStringResource = "Greet"
    @Parameter(title: "Name") var name: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: "Hello, \(name)")
    }
}

struct Shortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: Greet(), phrases: ["Greet in \(.applicationName)"], shortTitle: "Greet", systemImageName: "hand.wave")
    }
}

