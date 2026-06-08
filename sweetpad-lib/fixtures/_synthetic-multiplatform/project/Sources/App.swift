import SwiftUI

@main
struct MultiPlatformApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

struct ContentView: View {
    var body: some View {
        Text(Probe.greeting)
    }
}
