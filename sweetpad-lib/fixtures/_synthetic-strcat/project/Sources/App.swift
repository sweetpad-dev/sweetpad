import SwiftUI

@main
struct StringCatGenApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

struct ContentView: View {
    var body: some View {
        VStack(spacing: 8) {
            Text("StringCatGen")
            if #available(macOS 13, *) {
                Text(StringCatProbe.greeting)
                Text(StringCatProbe.itemsSummary(3))
            }
        }
        .padding()
    }
}
