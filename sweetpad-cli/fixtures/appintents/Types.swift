import AppIntents
import Foundation

enum SampleChoice: String, AppEnum {
    case first, second
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Choice"
    static let caseDisplayRepresentations: [SampleChoice: DisplayRepresentation] = [
        .second: "Second",
        .first: DisplayRepresentation(title: "First", subtitle: "First choice", image: .init(systemName: "1.circle")),
    ]
}

struct SampleEntity: AppEntity {
    let id: String
    @Property(title: "Name") var name: String
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Item"
    static let defaultQuery = SampleQuery()
    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "Item") }
}

struct SampleQuery: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [SampleEntity] { [] }
}

struct TypesIntent: AppIntent {
    static let title: LocalizedStringResource = "Types"
    static let description = IntentDescription("Metadata type coverage")
    static let openAppWhenRun = true

    @Parameter(title: "Text") var text: String?
    @Parameter(title: "Count", default: 3) var count: Int
    @Parameter(title: "Enabled", default: true) var enabled: Bool
    @Parameter(title: "Amount", default: 1.5) var amount: Double
    @Parameter(title: "URL") var url: URL
    @Parameter(title: "Date") var date: Date
    @Parameter(title: "Strings") var strings: [String]
    @Parameter(title: "Choice") var choice: SampleChoice
    @Parameter(title: "Item") var item: SampleEntity?

    func perform() async throws -> some IntentResult { .result() }
}

struct ConfigurationIntent: WidgetConfigurationIntent {
    static let title: LocalizedStringResource = "Configuration"
    @Parameter(title: "Choice", default: .first) var choice: SampleChoice
}

struct AuthenticatedIntent: AppIntent {
    static let title: LocalizedStringResource = "Authenticated"
    static let authenticationPolicy: IntentAuthenticationPolicy = .requiresAuthentication
    func perform() async throws -> some IntentResult { .result() }
}

struct LocalAuthenticationIntent: AppIntent {
    static let title: LocalizedStringResource = "Local authentication"
    static let authenticationPolicy: IntentAuthenticationPolicy = .requiresLocalDeviceAuthentication
    func perform() async throws -> some IntentResult { .result() }
}
