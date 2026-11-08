import Foundation
import Testing

@Test func testEnvironment() {
    #expect(ProcessInfo.processInfo.environment["MANIFEST_VALUE"] == "Linux tests ü")
}

@Test func testCommandLineArguments() {
    #expect(ProcessInfo.processInfo.arguments.contains("two words"))
}
