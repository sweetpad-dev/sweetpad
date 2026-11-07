import AppIntents
import SwiftUI
import ValueKit

@main
struct ParityProbeApp: App {
    var body: some Scene {
        WindowGroup {
            Text(ProbeValues.greeting).accessibilityIdentifier("greeting")
        }
    }
}

@MainActor
enum ProbeValues {
    static let source = "source-v1"
    static var greeting: String { "\(source)|\(ValueKit.message)" }
    static var origin: String {
        #if LINUX_PRODUCT
        "linux"
        #else
        "xcode"
        #endif
    }
    static func resource() throws -> String {
        guard let name = Bundle.main.object(forInfoDictionaryKey: "ProbeResourceName") as? String,
              let url = Bundle.main.url(forResource: name, withExtension: "txt") else {
            throw NSError(domain: "ParityProbe.MissingResource", code: 1)
        }
        return try String(contentsOf: url, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

struct ProbeIntent: AppIntent {
    static let title: LocalizedStringResource = "Parity probe"
    func perform() async throws -> some IntentResult { .result() }
}
