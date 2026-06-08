import Foundation

/// Resolves only when the editor analyzes this multiplatform target with a
/// concrete per-platform SDK — the `#if os(...)` branch a `-sdk auto` /
/// platform-mismatched `-target` would mis-evaluate or refuse to compile.
enum Probe {
    static let greeting = "Hello from \(platform)"

    static var platform: String {
        #if os(macOS)
        "macOS"
        #elseif os(iOS)
        "iOS"
        #else
        "other"
        #endif
    }
}
