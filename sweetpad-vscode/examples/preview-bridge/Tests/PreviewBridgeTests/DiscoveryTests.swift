import XCTest

@testable import PreviewBridge
import PreviewSamples

/// Exercises the `__swift5_proto` discovery against the known previews in the
/// PreviewSamples module. These are the assertions that catch a regression in
/// the conformance walk or in `fileID`/`line` extraction after an Xcode update.
@MainActor
final class DiscoveryTests: XCTestCase {
  override func setUp() {
    super.setUp()
    // Force the PreviewSamples image to stay loaded so its conformances are
    // visible to the runtime scan.
    XCTAssertEqual(PreviewSamplesAnchor.marker, "preview-samples")
  }

  private func sampleMacroPreviews() -> [DiscoveredPreview] {
    discoverPreviews().filter {
      $0.protocolName == "PreviewRegistry" && ($0.fileID?.hasSuffix("Samples.swift") ?? false)
    }
  }

  func testDiscoversBothPreviewKinds() {
    let all = discoverPreviews()
    XCTAssertTrue(all.contains { $0.protocolName == "PreviewRegistry" }, "Should find #Preview macros")
    XCTAssertTrue(
      all.contains { $0.protocolName == "PreviewProvider" && $0.typeName.contains("LegacySamplePreviews") },
      "Should find the legacy PreviewProvider",
    )
  }

  func testOnlyPreviewProtocolsAreReturned() {
    let protocols = Set(discoverPreviews().map(\.protocolName))
    XCTAssertTrue(protocols.isSubset(of: ["PreviewProvider", "PreviewRegistry"]), "Unexpected protocol: \(protocols)")
  }

  func testFindsTheTwoSampleMacros() {
    let macros = sampleMacroPreviews()
    XCTAssertEqual(macros.count, 2, "Expected exactly the two #Preview macros in Samples.swift")
  }

  func testMacroFileIDIsModuleQualified() {
    for preview in sampleMacroPreviews() {
      XCTAssertEqual(preview.fileID, "PreviewSamples/Samples.swift")
    }
  }

  func testMacroLinesArePresentAndDistinct() {
    let lines = sampleMacroPreviews().compactMap(\.line)
    XCTAssertEqual(lines.count, 2, "Both macros should report a source line")
    XCTAssertTrue(lines.allSatisfy { $0 > 0 })
    XCTAssertEqual(Set(lines).count, 2, "The two previews should be on different lines")
  }

  func testMacroDisplayNames() {
    let names = Set(sampleMacroPreviews().compactMap(\.displayName))
    XCTAssertEqual(names, ["Red Box", "Blue Box"])
  }
}
