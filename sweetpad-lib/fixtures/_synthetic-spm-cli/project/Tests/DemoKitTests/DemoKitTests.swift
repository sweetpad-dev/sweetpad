import XCTest

@testable import DemoKit

final class DemoKitTests: XCTestCase {
    func testGreeting() {
        XCTAssertEqual(greeting(), "hello from DemoKit")
    }
}
