import SwiftUI

@main
struct PreviewSampleApp: App {
  var body: some Scene {
    WindowGroup {
      // When SweetPad pins a preview via env vars, render it; otherwise boot
      // the normal app. This is the same shape the scaffolded bootstrap uses.
      if let preview = SweetPadPreviewHost.rootView() {
        preview
      } else {
        ContentView()
      }
    }
  }
}
