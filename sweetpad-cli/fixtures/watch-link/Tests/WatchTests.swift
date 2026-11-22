import WatchProbe
import XCTest

final class WatchTests: XCTestCase {
    func testValue() {
        XCTAssertEqual(probe(), 42)
    }
}
