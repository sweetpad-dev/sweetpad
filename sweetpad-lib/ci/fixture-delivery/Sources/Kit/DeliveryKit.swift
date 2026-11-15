import Foundation

/// The App Group the app and its widget share: 'group.' and the app's
/// bundle identifier, so the widget drops its own '.Widget'.
public enum Delivery {
    public static var groupIdentifier: String {
        var bundle = Bundle.main.bundleIdentifier ?? "dev.example.delivery"
        if bundle.hasSuffix(".Widget") {
            bundle.removeLast(".Widget".count)
        }
        return "group." + bundle
    }

    public static var shared: UserDefaults? {
        UserDefaults(suiteName: groupIdentifier)
    }

    public static func recordLaunch() -> Int {
        let count = (shared?.integer(forKey: "launches") ?? 0) + 1
        shared?.set(count, forKey: "launches")
        return count
    }

    public static var launches: Int {
        shared?.integer(forKey: "launches") ?? 0
    }

    /// The launch counts as a `Span` describes them. Before iOS and macOS
    /// 26 the `Span` type comes from the back-deployed
    /// libswiftCompatibilitySpan.dylib, so the app embeds that library and
    /// an App Store export carries its original in SwiftSupport.
    public static func describe(_ counts: [Int]) -> String {
        counts.withUnsafeBufferPointer { buffer in
            let span = Span(_unsafeElements: buffer)
            return "\(type(of: span)) of \(span.count)"
        }
    }
}
