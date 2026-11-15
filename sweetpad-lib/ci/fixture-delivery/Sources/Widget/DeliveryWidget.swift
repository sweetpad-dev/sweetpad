import SwiftUI
import WidgetKit

/// What the app recorded in the App Group it shares with this widget:
/// 'group.' and the app's bundle identifier, the widget's without '.Widget'.
enum Delivery {
    static var launches: Int {
        var bundle = Bundle.main.bundleIdentifier ?? "dev.example.delivery"
        if bundle.hasSuffix(".Widget") {
            bundle.removeLast(".Widget".count)
        }
        return UserDefaults(suiteName: "group." + bundle)?.integer(forKey: "launches") ?? 0
    }
}

struct LaunchEntry: TimelineEntry {
    let date: Date
    let launches: Int
}

struct LaunchProvider: TimelineProvider {
    func placeholder(in context: Context) -> LaunchEntry {
        LaunchEntry(date: .now, launches: 0)
    }

    func getSnapshot(in context: Context, completion: @escaping (LaunchEntry) -> Void) {
        completion(LaunchEntry(date: .now, launches: Delivery.launches))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<LaunchEntry>) -> Void) {
        completion(Timeline(entries: [LaunchEntry(date: .now, launches: Delivery.launches)], policy: .never))
    }
}

struct DeliveryWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "DeliveryWidget", provider: LaunchProvider()) { entry in
            Text("Launches: \(entry.launches)")
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Delivery")
        .description("How many times the app launched.")
    }
}

@main
struct DeliveryWidgets: WidgetBundle {
    var body: some Widget {
        DeliveryWidget()
    }
}
