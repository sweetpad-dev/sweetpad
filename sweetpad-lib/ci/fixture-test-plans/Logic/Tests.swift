import XCTest

final class PlanLogicTests: XCTestCase {
    func testOptions() {
        let process = ProcessInfo.processInfo
        XCTAssertEqual(process.environment["PLAN_SHARED"], "shared ü")
        XCTAssertEqual(process.environment["PLAN_PRODUCT"], "ProbeUITests")
        XCTAssertTrue(["first", "second", "region"].contains(process.environment["PLAN_VARIANT"] ?? ""))
        XCTAssertTrue(process.arguments.contains("two words"))
        if process.environment["PLAN_VARIANT"] == "region" {
            XCTAssertTrue(process.arguments.contains("en_FR"))
        } else {
            XCTAssertTrue(process.arguments.contains("(fr)") || process.arguments.contains("(en)"))
        }
    }
    func testExcluded() { XCTFail("test plan must exclude this test") }
}
