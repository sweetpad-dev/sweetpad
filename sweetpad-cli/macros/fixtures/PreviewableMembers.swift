// @Previewable variables whose initial values span several lines.

import SwiftUI

struct L: View {
    @Binding var items: [Int]
    var body: some View { Text("\(items.count)") }
}

#Preview {
    @Previewable @State var items = [
        1,
            2,
    ]
    L(items: $items)
}

#Preview {
        @Previewable @State var wide = [
          1,
        ]
    @Previewable
        @State var narrow = 0
    L(items: $wide)
}
