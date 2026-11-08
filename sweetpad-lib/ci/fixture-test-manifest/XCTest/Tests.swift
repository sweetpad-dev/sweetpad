import Foundation
import XCTest

final class LogicXCTests: XCTestCase {
    func testEnvironment() {
        XCTAssertEqual(ProcessInfo.processInfo.environment["MANIFEST_VALUE"], "Linux tests ü")
    }

    func testCommandLineArguments() {
        XCTAssertTrue(ProcessInfo.processInfo.arguments.contains("two words"))
    }
}
