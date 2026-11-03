// Run with swift; redirect stdout to appkit/font-names.tsv.
// Records system fonts only. Re-capture when updating the native oracle profile.
import AppKit
import CoreText
let urls = CTFontManagerCopyAvailableFontURLs() as! [URL]
var candidates = Set<String>()
for url in urls where url.path.hasPrefix("/System/Library/Fonts/") {
    guard let descriptors = CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as? [CTFontDescriptor] else { continue }
    for descriptor in descriptors {
        for key in [kCTFontNameAttribute, kCTFontDisplayNameAttribute, kCTFontFamilyNameAttribute] {
            if let name = CTFontDescriptorCopyAttribute(descriptor, key) as? String { candidates.insert(name) }
        }
    }
}
for name in candidates.sorted() {
    if let font = NSFont(name: name, size: 16) { print(name + "\t" + font.fontName) }
}
