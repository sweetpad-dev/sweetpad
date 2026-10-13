// #Predicate and #Expression over the operators and functions the macro
// knows.

import Foundation
import SwiftData

@Model final class Tag { var name: String; init(name: String) { self.name = name } }
@Model final class Note {
    var title: String
    var body: String?
    var count: Int
    var score: Double
    var tags: [Tag]
    var flags: [String]
    var created: Date
    init() { title = ""; body = nil; count = 0; score = 0; tags = []; flags = []; created = .now }
}

class Base { var x = 1 }
final class Sub: Base { var y = 2 }

func probes(limit: Int, word: String, date: Date) {
    let a = #Predicate<Note> { $0.count > limit }
    let b = #Predicate<Note> { $0.count >= 1 && $0.count <= 10 || !($0.score < 0.5) }
    let c = #Predicate<Note> { ($0.body ?? "") == word }
    let d = #Predicate<Note> { $0.body?.contains(word) == true }
    let e = #Predicate<Note> { $0.title.localizedStandardContains(word) }
    let f = #Predicate<Note> { $0.tags.contains { $0.name == word } }
    let g = #Predicate<Note> { $0.flags.contains(word) }
    let i = #Predicate<Note> { $0.tags.allSatisfy { !$0.name.isEmpty } }
    // The macro refuses this one and r: each is an error the plugin reports.
    let j = #Predicate<Note> { $0.created > date && $0.score * 2 + 1 > Double($0.count) }
    let k = #Predicate<Note> { note in
        if let body = note.body {
            body.count > 3
        } else {
            false
        }
    }
    let l = #Predicate<Note> { $0.count % 2 == 0 ? $0.score > 1 : -$0.score > 1 }
    let m = #Predicate<Note> { (1...5).contains($0.count) && (0..<3).contains($0.count) }
    let n = #Predicate<Note> { $0.body != nil && $0.body! != "" }
    let o = #Predicate<Note> { $0.flags.first == word && $0.flags[0] == word }
    let p = #Predicate<Base> { ($0 as? Sub)?.y == 2 && $0 is Sub }
    let q = #Predicate<Note> { a.evaluate($0) }
    let r = #Predicate<Note> { $0.title.caseInsensitiveCompare(word) == .orderedSame }
    let s = #Predicate<Note> { $0.flags.max() == word }
    let t = #Expression<Note, Int> { $0.count + 1 }
    let u = #Predicate<Note> { $0.score / 2 > 0.1 && $0.count / 2 > 1 }
    let v = #Predicate<String, Int> { s, n in s.count == n }
    let w = #Predicate<Note> { _ in true }
    _ = (a, b, c, d, e, f, g, i, j, k, l, m, n, o, p, q, r, s, t, u, v, w)
}
