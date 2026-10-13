// @Previewable where #Preview can't use it: in a type, after other
// statements, nested, in UIKit and argument previews. Some are compile
// errors.

import SwiftUI
import UIKit

struct B: View {
    @Binding var x: Int
    var body: some View { Text("\(x)") }
}

struct Holder: View {
    @Previewable @State var misplaced = 1
    var body: some View { Text("") }
}

#Preview {
    @Previewable let constant = 1
    @Previewable var plain = 2
    B(x: .constant(constant + plain))
}

#Preview {
    VStack {
        @Previewable @State var nested = 1
        B(x: $nested)
    }
}

#Preview {
    @Previewable @State var kit = 1
    UIView()
}

#Preview("Group", arguments: [1]) { n in
    @Previewable @State var grouped = 1
    B(x: $grouped)
}

#Preview {
    @Previewable @State var only = 1
}

#Preview {
    @Previewable @State var a = 1, b = 2
    B(x: $a)
}

#Preview {
    @Previewable @State var c: Int
    B(x: $c)
}

#Preview {
    @Previewable
    @State
    private var d = 1
    B(x: $d)
}
