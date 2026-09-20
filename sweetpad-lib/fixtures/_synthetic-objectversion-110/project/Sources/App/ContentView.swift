import SwiftUI

// Hot-reload self-check hook. `app run --hot --hot-selfcheck` rewrites the token
// below to a unique nonce and injects this file; the bundled client interposes
// this function, and the observer in SweetpadCIApp logs its value on the
// injection notification — so the self-check can confirm the *new* code actually
// ran, not just that the patch was accepted. Keep the token literal on one line.
func sweetpadHotReloadMarker() -> String { "SWEETPAD_MARKER_ORIGINAL" }

struct ContentView: View {
    var body: some View {
        Text("Hello, SweetPad CI")
            .padding()
    }
}
