import SwiftUI

@main
struct AssetSymApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

struct ContentView: View {
    var body: some View {
        VStack {
            AssetSymProbe.swatch
            AssetSymProbe.icon
        }
        .padding()
    }
}
