import AppIntents

struct WhenProbe: AppIntent {
    static let title: LocalizedStringResource = "Conditional"
    @Parameter(title: "Flag") var flag: Bool
    static var parameterSummary: some ParameterSummary {
        When(\.$flag, .equalTo, true) { Summary("On") } otherwise: { Summary("Off") }
    }
    func perform() async throws -> some IntentResult { .result() }
}

struct SwitchProbe: AppIntent {
    static let title: LocalizedStringResource = "Alternatives"
    @Parameter(title: "Number") var number: Int
    static var parameterSummary: some ParameterSummary {
        Switch(\.$number) {
            Case(1) { Summary("One") }
            DefaultCase { Summary("Other") }
        }
    }
    func perform() async throws -> some IntentResult { .result() }
}

struct TableProbe: AppIntent {
    static let title: LocalizedStringResource = "Localization table"
    static var parameterSummary: some ParameterSummary { Summary("From table", table: "Probe") }
    func perform() async throws -> some IntentResult { .result() }
}

struct RangeProbe: AppIntent {
    static let title: LocalizedStringResource = "Range"
    @Parameter(title: "Count", inclusiveRange: (1, 5)) var count: Int
    func perform() async throws -> some IntentResult { .result() }
}

struct Choices: DynamicOptionsProvider {
    func results() async throws -> [String] { ["First", "Second"] }
}

struct OptionsProviderProbe: AppIntent {
    static let title: LocalizedStringResource = "Choices"
    @Parameter(title: "Choice", optionsProvider: Choices()) var choice: String
    func perform() async throws -> some IntentResult { .result() }
}
