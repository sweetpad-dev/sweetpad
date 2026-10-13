// @Previewable variables with comments around them, tabs, one-line bodies,
// and an @Previewable outside any preview.

import SwiftUI

struct B: View {
    @Binding var x: Int
    var body: some View { Text("\(x)") }
}

#Preview {
    // leading
    @Previewable @State var x = 1 // trailing x
    // between
    @Previewable @State var y = 2

    // before body
    B(x: $x)
    B(x: $y) // end
}

#Preview {
	@Previewable @State var t = 1
	B(x: $t)
}

#Preview { @Previewable @State var z = 1; B(x: $z) }

#Preview("Body argument", body: {
    @Previewable @State var w = 1
    B(x: $w)
})

#Preview {
        @Previewable @State var deep = 1
        VStack {
            B(x: $deep)
        }
}

func outside() {
    @Previewable var v = 1
    _ = v
}
