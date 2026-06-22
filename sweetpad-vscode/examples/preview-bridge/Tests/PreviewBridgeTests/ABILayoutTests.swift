import XCTest

@testable import PreviewBridge

/// Pins the Swift ABI metadata layout the discovery relies on. If a future
/// Swift/Xcode toolchain changes these struct layouts, the `__swift5_proto`
/// walk would read garbage — these asserts fail first, with a precise reason.
final class ABILayoutTests: XCTestCase {
  func testProtocolConformanceDescriptorLayout() {
    XCTAssertEqual(MemoryLayout<ProtocolConformanceDescriptor>.size, 16, "Conformance descriptor is 4×Int32 fields")
    XCTAssertEqual(MemoryLayout<ProtocolConformanceDescriptor>.offset(of: \.protocolDescriptor), 0)
    XCTAssertEqual(MemoryLayout<ProtocolConformanceDescriptor>.offset(of: \.nominalTypeDescriptor), 4)
    XCTAssertEqual(MemoryLayout<ProtocolConformanceDescriptor>.offset(of: \.protocolWitnessTable), 8)
  }

  func testProtocolDescriptorLayout() {
    XCTAssertEqual(MemoryLayout<ProtocolDescriptor>.size, 24)
    XCTAssertEqual(MemoryLayout<ProtocolDescriptor>.offset(of: \.name), 8, "name follows flags(UInt32)+parent(Int32)")
  }

  func testModuleContextDescriptorLayout() {
    XCTAssertEqual(MemoryLayout<TargetModuleContextDescriptor>.size, 16)
    XCTAssertEqual(MemoryLayout<TargetModuleContextDescriptor>.offset(of: \.parent), 4)
    XCTAssertEqual(MemoryLayout<TargetModuleContextDescriptor>.offset(of: \.name), 8)
    XCTAssertEqual(MemoryLayout<TargetModuleContextDescriptor>.offset(of: \.accessFunction), 12)
  }

  func testContextDescriptorKindRawValues() {
    // These match the Swift runtime's ContextDescriptorKind enumeration.
    XCTAssertEqual(ContextDescriptorKind.Module.rawValue, 0)
    XCTAssertEqual(ContextDescriptorKind.Class.rawValue, 16)
    XCTAssertEqual(ContextDescriptorKind.Struct.rawValue, 17)
    XCTAssertEqual(ContextDescriptorKind.Enum.rawValue, 18)
  }
}
