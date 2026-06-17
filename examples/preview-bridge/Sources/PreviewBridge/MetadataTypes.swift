//
//  MetadataTypes.swift
//
//  Swift ABI metadata structures used to walk protocol-conformance records.
//
//  Adapted (with light, behavior-preserving edits) from EmergeTools/SnapshotPreviews,
//  which is MIT licensed: https://github.com/EmergeTools/SnapshotPreviews
//  Original author: Noah Martin.
//
//  These mirror the layout of Swift runtime metadata. They are inherently tied
//  to the Swift ABI — if the compiler changes these layouts, the regression
//  tests in Tests/PreviewBridgeTests/ABILayoutTests.swift are designed to fail
//  loudly so the breakage is caught after an Xcode/Swift update.

import Foundation

struct ProtocolConformanceDescriptor {
  let protocolDescriptor: Int32
  var nominalTypeDescriptor: Int32
  let protocolWitnessTable: Int32
  let conformanceFlags: ConformanceFlags
}

struct ProtocolDescriptor {
  let flags: UInt32
  let parent: Int32
  let name: Int32
  let numRequirementsInSignature: UInt32
  let numRequirements: UInt32
  let associatedTypeNames: Int32
}

public enum ContextDescriptorKind: UInt8 {
  case Module = 0
  case Extension = 1
  case Anonymous = 2
  case `Protocol` = 3
  case OpaqueType = 4
  case Class = 16
  case Struct = 17
  case Enum = 18
}

struct ContextDescriptorFlags {
  private let rawFlags: UInt32

  var kind: ContextDescriptorKind? {
    let value = UInt8(rawFlags & 0x1F)
    return ContextDescriptorKind(rawValue: value)
  }
}

struct TargetModuleContextDescriptor {
  let flags: ContextDescriptorFlags
  let parent: Int32
  let name: Int32
  let accessFunction: Int32
}

enum TypeReferenceKind: UInt32 {
  case DirectTypeDescriptor = 0
  case IndirectTypeDescriptor = 1
  case DirectObjCClassName = 2
  case IndirectObjCClass = 3
}

struct ConformanceFlags {
  private let rawFlags: UInt32

  var kind: TypeReferenceKind? {
    let rawKind = (rawFlags & Self.TypeMetadataKindMask) >> Self.TypeMetadataKindShift
    return TypeReferenceKind(rawValue: rawKind)
  }

  private static let TypeMetadataKindMask: UInt32 = 0x7 << Self.TypeMetadataKindShift
  private static let TypeMetadataKindShift: UInt32 = 3
}
