import TipKit
import SwiftUI

struct FixtureTip: Tip {
    @Parameter(.transient) static var enabled: Bool = false
    @Parameter static var count: Int = 3
    var title: Text { Text("Fixture") }
    var rules: [Rule] {
        #Rule(Self.$enabled) { $0 == true }
        #Rule(Self.$count) { $0 > 1 }
    }
}
