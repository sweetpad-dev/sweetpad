//
//  PreviewBridge.swift
//
//  Resolution layer on top of the `__swift5_proto` discovery: turns each
//  discovered conformance into a `DiscoveredPreview` carrying its source
//  location (fileID:line, for matching SweetPad's `path:line` ids) and a
//  renderable SwiftUI View.
//
//  Two paths:
//   - PreviewProvider (legacy): `type.previews` is PUBLIC API — no reflection.
//   - PreviewRegistry (#Preview macro): call `makePreview()` then extract the
//     View from `DeveloperToolsSupport.Preview` via Mirror + unsafeBitCast.
//     This is the version-fragile part guarded by the regression tests.

import SwiftUI
#if canImport(DeveloperToolsSupport)
import DeveloperToolsSupport
#endif

/// A SwiftUI preview discovered at runtime from the binary's metadata.
public struct DiscoveredPreview {
  /// Mangled Swift type name of the (generated) preview type.
  public let typeName: String
  /// `"PreviewProvider"` or `"PreviewRegistry"`.
  public let protocolName: String
  /// `#fileID` of the `#Preview` (e.g. `MyModule/ContentView.swift`); nil for legacy.
  public let fileID: String?
  /// 1-based source line of the `#Preview`; nil for legacy `PreviewProvider`.
  public let line: Int?
  /// Optional display name (the `#Preview("…")` label).
  public let displayName: String?
  /// Builds the preview's root view. nil when extraction failed.
  public let makeView: (@MainActor () -> any View)?
}

/// Discover every `#Preview` / `PreviewProvider` in the current process.
@MainActor
public func discoverPreviews() -> [DiscoveredPreview] {
  getPreviewTypes().compactMap { resolve($0) }
}

@MainActor
private func resolve(_ result: LookupResult) -> DiscoveredPreview? {
  let metatype = unsafeBitCast(result.accessor(), to: Any.Type.self)

  if result.proto == "PreviewProvider" {
    guard let provider = metatype as? any PreviewProvider.Type else { return nil }
    return DiscoveredPreview(
      typeName: result.name,
      protocolName: result.proto,
      fileID: nil,
      line: nil,
      displayName: nil,
      makeView: { providerView(provider) },
    )
  }

  if result.proto == "PreviewRegistry", #available(macOS 14.0, iOS 17.0, *) {
    guard let registry = metatype as? any PreviewRegistry.Type else { return nil }
    let view = registryView(registry)
    return DiscoveredPreview(
      typeName: result.name,
      protocolName: result.proto,
      fileID: registry.fileID,
      line: registry.line,
      displayName: previewDisplayName(registry),
      makeView: view,
    )
  }

  return nil
}

@MainActor
private func providerView(_ provider: any PreviewProvider.Type) -> any View {
  provider.previews
}

@available(macOS 14.0, iOS 17.0, *)
@MainActor
private func previewDisplayName(_ registry: any PreviewRegistry.Type) -> String? {
  guard let preview = try? registry.makePreview() else { return nil }
  return Mirror(reflecting: preview).descendant("displayName") as? String
}

/// Extract a renderable view from a `#Preview` macro type.
///
/// The `#Preview` macro generates a `PreviewRegistry` whose `makePreview()`
/// returns a `DeveloperToolsSupport.Preview`. That type exposes no public way
/// to reach the view, so we reflect into it:
///
///   Preview → source → structure → singlePreview → makeBody (an opaque closure)
///
/// and `unsafeBitCast` `makeBody` to `@MainActor () -> any View`. The exact
/// child labels are an Xcode/SwiftUI implementation detail — RenderingTests
/// pins them so an Xcode update that changes them fails clearly.
@available(macOS 14.0, iOS 17.0, *)
@MainActor
private func registryView(_ registry: any PreviewRegistry.Type) -> (@MainActor () -> any View)? {
  // Defer the fragile reflection to call time so plain discovery never touches
  // it (and never risks a trap). Re-derive on each call so the closure's
  // captured context stays alive (owned by `preview`) while we invoke it.
  return { @MainActor in
    guard let preview = try? registry.makePreview(),
          let body = makeBodyChild(of: preview),
          let view = invokeMakeBody(body)
    else {
      return AnyView(EmptyView())
    }
    return view
  }
}

/// `boxed` is an `Any` wrapping the preview's `@MainActor () -> any View`
/// `makeBody` closure. A closure is two words and is stored inline at the start
/// of the `Any` existential buffer, so we read it back out and invoke it while
/// `boxed` (which retains the closure's context) is still alive. Using a sized
/// load instead of `unsafeBitCast(Any, …)` avoids the size-mismatch trap.
@available(macOS 14.0, iOS 17.0, *)
@MainActor
func invokeMakeBody(_ boxed: Any) -> (any View)? {
  typealias Make = @MainActor () -> any View
  guard MemoryLayout<Make>.size <= MemoryLayout<Any>.size else { return nil }
  let make = withUnsafeBytes(of: boxed) { raw -> Make in
    raw.baseAddress!.assumingMemoryBound(to: Make.self).pointee
  }
  return make()
}

/// Walk `source → structure → singlePreview → makeBody` and return the raw
/// `makeBody` value (still type-erased). Exposed to tests so the reflection
/// path can be asserted step-by-step.
@available(macOS 14.0, iOS 17.0, *)
func makeBodyChild(of preview: Any) -> Any? {
  func child(_ mirror: Mirror, _ label: String) -> Mirror? {
    mirror.children.first { $0.label == label }.map { Mirror(reflecting: $0.value) }
  }
  let root = Mirror(reflecting: preview)
  guard let source = child(root, "source"),
        let structure = child(source, "structure"),
        let single = child(structure, "singlePreview"),
        let makeBody = single.children.first(where: { $0.label == "makeBody" })
  else {
    return nil
  }
  return makeBody.value
}
