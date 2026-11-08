import XCTest

final class ProbeUITests: XCTestCase {
    func testTap() {
        let process = ProcessInfo.processInfo
        XCTAssertEqual(process.environment["PLAN_SHARED"], "shared ü")
        XCTAssertTrue(["first", "second"].contains(process.environment["PLAN_VARIANT"] ?? ""))
        XCTAssertEqual(process.environment["PLAN_PRODUCT"], "ProbeUITests")
        XCTAssertTrue(process.arguments.contains("two words"))
        let app = XCUIApplication()
        app.launch()
        let button = app.buttons["probe"]
        guard button.waitForExistence(timeout: 10) else { XCTFail("probe button missing"); return }
        button.tap()
        XCTAssertEqual(button.label, "Passed")
    }
    func testExcluded() { XCTFail("test-plan selection must exclude this test") }
}
