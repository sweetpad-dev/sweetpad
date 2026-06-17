import SwiftUI

/// A full-screen solid-color swatch with a big label. Distinct swatches produce
/// visually distinct screenshots, which the CI pipeline uses to prove that
/// preview selection (by id) and appearance (light/dark) actually take effect.
struct SwatchView: View {
  @Environment(\.colorScheme) private var colorScheme
  let label: String
  let color: Color

  var body: some View {
    ZStack {
      // Dark mode darkens the swatch so light/dark screenshots differ.
      color
        .brightness(colorScheme == .dark ? -0.4 : 0)
        .ignoresSafeArea()
      Text(label)
        .font(.system(size: 120, weight: .bold))
        .foregroundStyle(.white)
    }
  }
}

/// The app's normal root view (shown when SweetPad isn't pinning a preview).
struct ContentView: View {
  var body: some View {
    SwatchView(label: "Home", color: .green)
  }
}

#Preview("A") {
  SwatchView(label: "A", color: .red)
}

#Preview("B") {
  SwatchView(label: "B", color: .blue)
}
