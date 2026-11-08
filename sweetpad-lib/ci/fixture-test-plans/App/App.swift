import SwiftUI

@main
struct ProbeApp: App {
    var body: some Scene {
        WindowGroup { ProbeView() }
    }
}

struct ProbeView: View {
    @State private var title = "Ready"
    var body: some View {
        Button(title) { title = "Passed" }
            .accessibilityIdentifier("probe")
    }
}
