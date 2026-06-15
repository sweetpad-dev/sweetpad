import SwiftUI

// Minimal native-macOS app. The DerivedData oracle only runs
// `-showBuildSettings`, so this never has to compile a real UI — it just gives
// XcodeGen a source to attach to the target.
@main
struct DDProbeApp: App {
    var body: some Scene {
        WindowGroup {
            Text("DerivedData oracle probe")
        }
    }
}
