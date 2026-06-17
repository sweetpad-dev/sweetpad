import SwiftUI

/// A fixed-size solid-color box. ImageRenderer needs a concrete size, and a
/// solid fill makes the rendered output trivially checkable (the average pixel
/// equals the color), so the tests can prove "preview X → the right view".
public struct ColorBox: View {
  let color: Color
  public init(_ color: Color) {
    self.color = color
  }
  public var body: some View {
    color.frame(width: 120, height: 120)
  }
}

/// Referenced from the test target so the linker keeps this module's image
/// loaded (and its preview conformances discoverable) under `swift test`.
public enum PreviewSamplesAnchor {
  public static let marker = "preview-samples"
}

// Two #Preview macros (PreviewRegistry conformances) with distinct colors and
// labels — the tests assert these are discovered with the right fileID/line and
// render to the right color.
#Preview("Red Box") {
  ColorBox(.red)
}

#Preview("Blue Box") {
  ColorBox(.blue)
}

// A legacy PreviewProvider (rendered via the public `previews` API).
struct LegacySamplePreviews: PreviewProvider {
  static var previews: some View {
    ColorBox(.green)
  }
}
