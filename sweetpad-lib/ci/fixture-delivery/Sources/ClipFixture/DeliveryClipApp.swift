import SwiftUI

@main
struct DeliveryClipApp: App {
    @State private var invocation = ""

    var body: some Scene {
        WindowGroup {
            VStack {
                Text(Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "Delivery")
                Text(invocation)
            }
            .onContinueUserActivity(NSUserActivityTypeBrowsingWeb) { activity in
                if let url = activity.webpageURL { invocation = url.absoluteString }
            }
            .onOpenURL { url in invocation = url.absoluteString }
        }
    }
}
