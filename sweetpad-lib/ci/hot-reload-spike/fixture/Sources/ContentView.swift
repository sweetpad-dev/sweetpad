import SwiftUI

// The file the spike recompiles + injects. The `marker` value is bumped by the
// harness before injection; a successful `.injected` means the client loaded and
// patched the recompiled image. (SwiftUI redraw needs @ObserveInjection, which
// the spike intentionally omits — it asserts on the socket signal, not pixels.)
struct ContentView: View {
    static let marker = 1

    var body: some View {
        Text("HotReloadSpike marker=\(Self.marker)")
            .padding()
    }
}
