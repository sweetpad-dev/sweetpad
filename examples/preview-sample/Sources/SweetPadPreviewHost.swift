import SwiftUI

/// Reference implementation of the contract SweetPad's extension drives: read
/// `SWEETPAD_PREVIEW_ID` / `SWEETPAD_PREVIEW_APPEARANCE` from the environment and
/// render the selected preview, so the simulator can be streamed to VSCode.
///
/// This sample uses an explicit registry keyed by a logical id ("A"/"B") so the
/// CI pipeline is deterministic. The real extension passes a `path:line` id; an
/// app can map that to a view via EmergeTools/SnapshotPreviews discovery (see
/// the snapshot-check probe) or its own registry like this one.
enum SweetPadPreviewHost {
  static var requestedId: String? {
    ProcessInfo.processInfo.environment["SWEETPAD_PREVIEW_ID"]
  }

  static var appearance: ColorScheme? {
    switch ProcessInfo.processInfo.environment["SWEETPAD_PREVIEW_APPEARANCE"] {
    case "dark": return .dark
    case "light": return .light
    default: return nil
    }
  }

  @MainActor static let registry: [String: () -> AnyView] = [
    "A": { AnyView(SwatchView(label: "A", color: .red)) },
    "B": { AnyView(SwatchView(label: "B", color: .blue)) },
  ]

  /// Returns the requested preview view, or nil so the app boots normally.
  @MainActor static func rootView() -> AnyView? {
    guard let id = requestedId, let make = registry[id] else { return nil }
    let view = make()
    if let scheme = appearance {
      return AnyView(view.preferredColorScheme(scheme))
    }
    return view
  }
}
