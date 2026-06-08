import XCTest

@testable import Lib

/// Resolves only when the BSP gives a test file the unit-test search paths:
/// `import XCTest` needs the platform's test-framework `-F`, and the
/// `@testable import Lib` + `Lib.value` reference needs the framework-under-test's
/// products `-F` with `Lib.swiftmodule` built (build-for-testing).
final class ProbeTests: XCTestCase {
    func testValue() {
        XCTAssertEqual(Lib.value, 42)
    }
}
