import DeliveryKit
import SwiftUI

@main
struct DeliveryApp: App {
    @State private var launches = Delivery.recordLaunch()

    var body: some Scene {
        WindowGroup {
            VStack(spacing: 12) {
                Text("Delivery").font(.largeTitle)
                Text("Launches recorded in \(Delivery.groupIdentifier): \(launches)")
            }
            .padding()
        }
    }
}
