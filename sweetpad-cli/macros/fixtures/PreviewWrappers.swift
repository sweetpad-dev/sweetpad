// Which property wrappers #Preview warns about on a local variable.

import SwiftUI
import SwiftData
import UIKit

final class Model: ObservableObject {}
struct V: View { var body: some View { Text("") } }

#Preview {
    @State var a = 1
    @FocusState var c: Bool
    @StateObject var d = Model()
    @ObservedObject var e = Model()
    @Environment(\.colorScheme) var f
    @EnvironmentObject var g: Model
    @AppStorage("k") var h = 1
    @SceneStorage("s") var i = 1
    @Namespace var j
    @GestureState var k = 1
    @AccessibilityFocusState var l: Bool
    @ScaledMetric var m = 1.0
    @Bindable var n = Model2()
    @Query var p: [Item]
    @Custom var q = 1
    let r = 1
    V()
}

@Observable final class Model2 {}
@Model final class Item { init() {} }
@propertyWrapper struct Custom { var wrappedValue: Int }
extension FocusedValues { var `self`: Int? { nil } }

#Preview {
    @Previewable @State var x = 1
    VStack {
        V().previewLayout(.sizeThatFits)
    }
    .previewDisplayName("n")
}

#Preview {
}

#Preview {
    V().previewContext(WidgetPreviewContext())
}

#Preview("Group", arguments: [1]) { n in
    @State var inGroup = 1
    V().previewLayout(.sizeThatFits)
}

#Preview {
    VStack {
        @State var nested = 1
        V()
    }
}
struct WidgetPreviewContext: PreviewContext { typealias Key = Never; subscript<K>(key: K.Type) -> K.Value where K : PreviewContextKey { fatalError() } }
