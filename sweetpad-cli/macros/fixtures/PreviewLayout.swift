// How a #Preview body's own layout carries into the expansion: comments,
// blank lines, other indentation, tabs, a body passed in parentheses, and a
// #Preview inside a type.

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

#Preview {
    // a comment

    V()   // trailing

    V(n: 2)
}

#Preview {
  VStack {
      V()
  }
}

#Preview("Paren", body: { V() })

#Preview { V(); V(n: 1) }

#Preview(
    "Multi",
    traits: .landscapeLeft,
) {
    V()
}

#Preview {
    // leading
    @Previewable @State var x = 1
    let unused = 3
    @Previewable @State var y = 2 // trailing
    // between
    B(x: $x)
    B(x: $y)
}

#Preview("Group", arguments: [1, 2]) { (n: Int) in
    // comment
    V(n: n)
}

#Preview("Group2", arguments: [1, 2]) {
    n in V(n: n)
}

#Preview("Group3", arguments: [1, 2]) { V(n: $0) }

#Preview { UIView() as UIView }

#Preview {
	UIView()
}

@available(iOS 18, *)
struct Holder {
    #Preview { V() }
}
