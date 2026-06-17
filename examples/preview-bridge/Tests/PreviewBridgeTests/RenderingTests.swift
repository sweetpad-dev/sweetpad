import SwiftUI
import XCTest

@testable import PreviewBridge
import PreviewSamples

/// Renders discovered previews and asserts the right view came out. This is the
/// most Xcode-fragile layer: the #Preview path depends on the SwiftUI SPI shape
/// (`PreviewRegistry.makePreview()`) and the private `Preview` reflection chain
/// (source → structure → singlePreview → makeBody). A failure here after an
/// Xcode update means that SPI/reflection contract changed.
@MainActor
final class RenderingTests: XCTestCase {
  override func setUp() {
    super.setUp()
    XCTAssertEqual(PreviewSamplesAnchor.marker, "preview-samples")
  }

  private func preview(named name: String) throws -> DiscoveredPreview {
    let match = discoverPreviews().first { $0.displayName == name }
    return try XCTUnwrap(match, "No discovered preview named \(name)")
  }

  private func legacyPreview() throws -> DiscoveredPreview {
    let match = discoverPreviews().first {
      $0.protocolName == "PreviewProvider" && $0.typeName.contains("LegacySamplePreviews")
    }
    return try XCTUnwrap(match, "Legacy PreviewProvider not found")
  }

  /// Assert one channel clearly dominates (a solid red/green/blue fill).
  private func assertDominant(_ channel: KeyPath<AverageColor, UInt8>, _ color: AverageColor, _ label: String) {
    let value = color[keyPath: channel]
    let others: [KeyPath<AverageColor, UInt8>] = [\.r, \.g, \.b]
      .filter { $0 != channel }
    let otherValues = others.map { color[keyPath: $0] }
    XCTAssertGreaterThan(value, 120, "\(label): dominant channel too dark \(color)")
    for other in otherValues {
      XCTAssertGreaterThan(Int(value) - Int(other), 30, "\(label): channel not dominant enough \(color)")
    }
  }

  func testLegacyProviderRendersGreen() throws {
    let view = try XCTUnwrap(legacyPreview().makeView, "PreviewProvider.previews should always be renderable (public API)")
    let color = try XCTUnwrap(renderAverageColor(view()), "ImageRenderer produced no image")
    assertDominant(\.g, color, "legacy green")
  }

  func testMacroExtractionAndRedRender() throws {
    let red = try preview(named: "Red Box")
    let view = try XCTUnwrap(
      red.makeView,
      "View extraction failed — the Preview reflection chain (source→structure→singlePreview→makeBody) likely changed in this Xcode",
    )
    let color = try XCTUnwrap(renderAverageColor(view()), "ImageRenderer produced no image")
    assertDominant(\.r, color, "macro red")
  }

  func testMacroBlueRender() throws {
    let blue = try preview(named: "Blue Box")
    let view = try XCTUnwrap(blue.makeView, "View extraction failed for Blue Box")
    let color = try XCTUnwrap(renderAverageColor(view()), "ImageRenderer produced no image")
    assertDominant(\.b, color, "macro blue")
  }

  func testSelectionIsDistinct() throws {
    let red = try XCTUnwrap(preview(named: "Red Box").makeView)
    let blue = try XCTUnwrap(preview(named: "Blue Box").makeView)
    let redColor = try XCTUnwrap(renderAverageColor(red()))
    let blueColor = try XCTUnwrap(renderAverageColor(blue()))
    XCTAssertNotEqual(redColor, blueColor, "Different preview ids must render different views")
  }
}
