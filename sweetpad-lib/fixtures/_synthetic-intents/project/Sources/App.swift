import SwiftUI

@main
struct IntentsGenApp: App {
    var body: some Scene {
        WindowGroup {
            Text("IntentsGen \(IntentsProbe.intent.name ?? "")")
        }
    }
}
