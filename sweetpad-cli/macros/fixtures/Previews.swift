// #Preview in each form the SDK declares: SwiftUI views, with arguments,
// with @Previewable state, UIKit views and view controllers, and widgets.

import ActivityKit
import AppIntents
import SwiftUI
import UIKit
import WidgetKit

struct V: View {
    var n = 0
    var body: some View { Text("\(n)") }
}

struct B: View {
    @Binding var x: Int
    var body: some View { Text("\(x)") }
}

#Preview { V() }

#Preview("Named") { V() }

#Preview("Traits", traits: .landscapeLeft) { V() }

#Preview(traits: .sizeThatFitsLayout, .fixedLayout(width: 10, height: 10)) {
    V()
}

#Preview {
    let n = 3
    return V(n: n)
}

#Preview {
    V()
    V(n: 1)
}

#Preview {
    @Previewable @State var x = 1
    B(x: $x)
}

#Preview("Previewable", traits: .landscapeLeft) {
    @Previewable @State var x: Int = 1
    @Previewable @FocusState var focused: Bool
    let y = 2
    B(x: $x)
        .focused($focused)
        .padding(CGFloat(y))
}

#Preview("Arguments", arguments: [1, 2]) { n in
    V(n: n)
}

#Preview { UIView() }

#Preview("Kit", traits: .landscapeLeft) { UIViewController() }

#Preview {
    let label = UILabel()
    label.text = "x"
    return label
}

#Preview("Kit arguments", arguments: [1, 2]) { n in UIView() }

#Preview("Kit view controller arguments", traits: .portrait, arguments: ["a"]) { s in
    UIViewController()
}

struct Entry: TimelineEntry {
    var date: Date
}

struct Provider: TimelineProvider {
    func placeholder(in context: Context) -> Entry { Entry(date: .now) }
    func getSnapshot(in context: Context, completion: @escaping (Entry) -> Void) {
        completion(Entry(date: .now))
    }
    func getTimeline(in context: Context, completion: @escaping (Timeline<Entry>) -> Void) {
        completion(Timeline(entries: [Entry(date: .now)], policy: .never))
    }
}

struct Small: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "small", provider: Provider()) { _ in Text("") }
    }
}

#Preview(as: .systemSmall) {
    Small()
} timeline: {
    Entry(date: .now)
    Entry(date: .distantFuture)
}

#Preview("Provider", as: .systemMedium) {
    Small()
} timelineProvider: {
    Provider()
}

struct Choice: WidgetConfigurationIntent {
    static let title: LocalizedStringResource = "Choice"
    init() {}
}

struct IntentProvider: AppIntentTimelineProvider {
    func placeholder(in context: Context) -> Entry { Entry(date: .now) }
    func snapshot(for configuration: Choice, in context: Context) async -> Entry { Entry(date: .now) }
    func timeline(for configuration: Choice, in context: Context) async -> Timeline<Entry> {
        Timeline(entries: [Entry(date: .now)], policy: .never)
    }
}

struct Configurable: Widget {
    var body: some WidgetConfiguration {
        AppIntentConfiguration(kind: "configurable", intent: Choice.self, provider: IntentProvider()) { _ in
            Text("")
        }
    }
}

#Preview(as: .systemSmall, using: Choice()) {
    Configurable()
} timelineProvider: {
    IntentProvider()
}

struct Delivery: ActivityAttributes {
    struct ContentState: Codable, Hashable {
        var minutes: Int
    }
    var name: String
}

struct Live: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: Delivery.self) { _ in
            Text("")
        } dynamicIsland: { _ in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) { Text("") }
            } compactLeading: {
                Text("")
            } compactTrailing: {
                Text("")
            } minimal: {
                Text("")
            }
        }
    }
}

#Preview("Live", as: .content, using: Delivery(name: "x")) {
    Live()
} contentStates: {
    Delivery.ContentState(minutes: 1)
    Delivery.ContentState(minutes: 2)
}
