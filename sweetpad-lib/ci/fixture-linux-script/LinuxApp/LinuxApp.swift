import SwiftUI

@main
struct LinuxApp: App {
    init() {
        verifyEntryDefaults()
    }
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}
