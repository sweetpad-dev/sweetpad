import SwiftUI
import os

@main
struct SweetpadCIApp: App {
    init() {
        // On each successful injection the client posts INJECTION_BUNDLE_NOTIFICATION
        // (the same signal the Inject package observes). Log the interposed marker
        // so `--hot-selfcheck` can confirm the new code ran via the unified log.
        _ = NotificationCenter.default.addObserver(
            forName: Notification.Name("INJECTION_BUNDLE_NOTIFICATION"),
            object: nil,
            queue: nil
        ) { _ in
            os_log("SWEETPAD_HOTRELOAD %{public}@", sweetpadHotReloadMarker())
        }
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}
