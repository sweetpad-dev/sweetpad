import XCTest

final class DeliveryClipInvocationTests: XCTestCase {
    func testInstalledClipDisplaysInvocationURL() throws {
        let clip = XCUIApplication(bundleIdentifier: "dev.example.delivery.Clip")
        let url = try XCTUnwrap(URL(string: "https://example.com/delivery/\(UUID().uuidString)"))
        clip.launch()
        XCTAssertTrue(clip.staticTexts["Delivery Clip"].waitForExistence(timeout: 10))
        clip.open(url)
        XCTAssertTrue(clip.staticTexts[url.absoluteString].waitForExistence(timeout: 10))
    }
}
