import SwiftUI

#if canImport(PreviewGallery)
import PreviewGallery
#endif

/// Probe app: confirms EmergeTools/SnapshotPreviews' `PreviewGallery` product
/// resolves and compiles on the runner's real Swift toolchain. If the module or
/// type name differs in the pinned version, this build fails (non-blocking in
/// CI) and the log tells us the correct API to wire into the extension scaffold.
@main
struct SnapshotCheckApp: App {
  var body: some Scene {
    WindowGroup {
      #if canImport(PreviewGallery)
      PreviewGallery()
      #else
      Text("PreviewGallery not available")
      #endif
    }
  }
}

#Preview("Probe") {
  Text("SnapshotPreviews probe")
}
