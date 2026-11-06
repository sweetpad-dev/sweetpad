//
//  P+CoreDataProperties.swift
//  
//
//  Created by $(AUTHOR) on $(DATE).
//
//  This file was automatically generated and should not be edited.
//

public import Foundation
public import CoreData


public typealias PCoreDataPropertiesSet = NSSet

extension P {

    @nonobjc public class func fetchRequest() -> NSFetchRequest<P> {
        return NSFetchRequest<P>(entityName: "P")
    }

    @NSManaged nonisolated public var bin: Data?
    @NSManaged nonisolated public var uid: UUID?
    @NSManaged nonisolated public var day: Date?
    @NSManaged nonisolated public var yes: Bool
    @NSManaged nonisolated public var d: Double
    @NSManaged nonisolated public var i: Int32
    @NSManaged nonisolated public var l: Int64
    @NSManaged nonisolated public var s: String?
    @NSManaged nonisolated public var plain: Int32
    @NSManaged nonisolated public var q: Q?
    @NSManaged nonisolated public var rs: NSSet?
    @NSManaged nonisolated public var qs: NSOrderedSet?

}

// MARK: Generated accessors for rs
extension P {

    @objc(addRsObject:)
    @NSManaged nonisolated public func addToRs(_ value: R)

    @objc(removeRsObject:)
    @NSManaged nonisolated public func removeFromRs(_ value: R)

    @objc(addRs:)
    @NSManaged nonisolated public func addToRs(_ values: NSSet)

    @objc(removeRs:)
    @NSManaged nonisolated public func removeFromRs(_ values: NSSet)

}

// MARK: Generated accessors for qs
extension P {

    @objc(insertObject:inQsAtIndex:)
    @NSManaged nonisolated public func insertIntoQs(_ value: Q, at idx: Int)

    @objc(removeObjectFromQsAtIndex:)
    @NSManaged nonisolated public func removeFromQs(at idx: Int)

    @objc(insertQs:atIndexes:)
    @NSManaged nonisolated public func insertIntoQs(_ values: [Q], at indexes: NSIndexSet)

    @objc(removeQsAtIndexes:)
    @NSManaged nonisolated public func removeFromQs(at indexes: NSIndexSet)

    @objc(replaceObjectInQsAtIndex:withObject:)
    @NSManaged nonisolated public func replaceQs(at idx: Int, with value: Q)

    @objc(replaceQsAtIndexes:withQs:)
    @NSManaged nonisolated public func replaceQs(at indexes: NSIndexSet, with values: [Q])

    @objc(addQsObject:)
    @NSManaged nonisolated public func addToQs(_ value: Q)

    @objc(removeQsObject:)
    @NSManaged nonisolated public func removeFromQs(_ value: Q)

    @objc(addQs:)
    @NSManaged nonisolated public func addToQs(_ values: NSOrderedSet)

    @objc(removeQs:)
    @NSManaged nonisolated public func removeFromQs(_ values: NSOrderedSet)

}

extension P : Identifiable {

}
