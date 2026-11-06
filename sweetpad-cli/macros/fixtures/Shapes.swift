// Declarations at the edges of what @Model, @Query and @ModelActor take.
// Some are compile errors (a 'let', a 'lazy' or a 'static' persisted
// property); Xcode's plugin expands them all the same, and so must ours.

import Foundation
import SwiftData
import SwiftUI

@Model
public final class Pub {
    public var name: String
    public init(name: String) { self.name = name }
}

@Model
final class Empty {
    init() {}
}

@Model
final class Mixed {
    #Unique<Mixed>([\.z])
    var x: Int? = 5
    let y: String
    var z = "s"
    static var s = 1
    @Attribute var plain: String = ""
    @Attribute(.unique, originalName: "o") var both: String = ""
    @Relationship var others: [Pub] = []
    var dict: [String: Int] = [:]
    var opt: Optional<Int>
    var maybe: Int! = nil
    lazy var lz: Int = 3
    var observed: Int = 0 { didSet { print("x") } }
    init(y: String) { self.y = y; self.opt = nil }
}

enum Outer {
    @Model
    final class Inner {
        var v: Int = 1
        init() {}
    }
}

@available(iOS 26, *)
@Model
class Base {
    var a: Int = 0
    init() {}
}

@available(iOS 26, *)
@Model
final class Sub: Base {
    var b: Int = 0
    override init() { super.init() }
}

@ModelActor
public actor PubActor {}

struct V: View {
    @Query(FetchDescriptor<Pub>()) var a: [Pub]
    @Query(sort: [SortDescriptor(\Pub.name)], animation: .default) var b: [Pub]
    @Query(sort: \Pub.name, order: .reverse) private var c: [Pub]
    var body: some View { Text("") }
}
