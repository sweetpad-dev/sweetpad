// Run with swift; redirect stdout to appkit/layout-fonts.json.
// Captures metrics, not font files. Extend the profile with native oracle tests.
import AppKit
let _ = NSApplication.shared
let characters = (32...126).compactMap(UnicodeScalar.init).map(String.init) + ["…", "–", "—", "’", "“", "”", "•", "é", "⌘"]
var profiles: [String: Any] = [:]
for (name, size, weight) in [("regular", 13.0, NSFont.Weight.regular), ("small", 11.0, .regular), ("mini", 9.0, .regular), ("bold", 13.0, .bold), ("smallBold", 11.0, .bold), ("regular12", 12.0, .regular), ("regular15", 15.0, .regular), ("regular16", 16.0, .regular), ("bold16", 16.0, .bold), ("bold15", 15.0, .bold), ("thin36", 36.0, .thin)] {
    let font = NSFont.systemFont(ofSize: size, weight: weight)
    func width(_ text: String) -> Double { NSAttributedString(string:text,attributes:[.font:font]).size().width }
    var advances: [String:Double] = [:], kerns: [String:Double] = [:]
    for a in characters { advances[a] = width(a) }
    for a in characters { for b in characters {
        let kern = width(a+b) - advances[a]! - advances[b]!
        if abs(kern) > 0.00001 { kerns[a+b] = kern }
    } }
    let label = NSTextField(labelWithString:"Hello");label.font=font
    profiles[name] = ["size":size, "height":label.intrinsicContentSize.height, "advances":advances, "kerns":kerns]
}
let data = try JSONSerialization.data(withJSONObject:profiles,options:[.prettyPrinted,.sortedKeys])
FileHandle.standardOutput.write(data)
