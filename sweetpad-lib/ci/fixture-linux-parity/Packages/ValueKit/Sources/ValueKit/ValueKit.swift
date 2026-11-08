import Foundation

public enum ValueKit {
    public static let message = "package-v1"
    public static func resource() throws -> String {
        guard let url = Bundle.module.url(forResource: "package", withExtension: "txt") else {
            throw NSError(domain: "ValueKit.MissingResource", code: 1)
        }
        return try String(contentsOf: url, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
