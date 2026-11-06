// What #Preview warns about: property wrappers on local variables and the
// PreviewProvider modifiers it ignores.

import SwiftUI

struct B: View {
    @Binding var x: Int
    var body: some View { Text("\(x)") }
}

#Preview {
    @State var missing = 1
    B(x: $missing)
}

#Preview {
    Text("").previewLayout(.sizeThatFits).previewDisplayName("x").previewInterfaceOrientation(.landscapeLeft).previewDevice("iPhone 15")
}

#Preview {
    @Previewable @State var x = 1
    Button("b") { x += 1 }.onAppear { x = 2 }
}

@available(iOS, unavailable)
struct H1 {
    #Preview { Text("1") }
}

@available(iOS, deprecated)
struct H2 {
    #Preview { Text("2") }
}

@available(*, unavailable)
struct H3 {
    #Preview { Text("3") }
}

@available(macOS 15, *)
struct H4 {
    #Preview { Text("4") }
}

@available(iOS 17, *)
extension B {
    #Preview { Text("5") }
}
