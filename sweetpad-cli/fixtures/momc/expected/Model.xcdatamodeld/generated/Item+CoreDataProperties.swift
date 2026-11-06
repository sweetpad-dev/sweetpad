//
//  Item+CoreDataProperties.swift
//  
//
//  Created by $(AUTHOR) on $(DATE).
//
//  This file was automatically generated and should not be edited.
//

public import Foundation
public import CoreData


public typealias ItemCoreDataPropertiesSet = NSSet

extension Item {

    @nonobjc public class func fetchRequest() -> NSFetchRequest<Item> {
        return NSFetchRequest<Item>(entityName: "Item")
    }

    @NSManaged nonisolated public var count: Int64
    @NSManaged nonisolated public var created: Date?
    @NSManaged nonisolated public var name: String?
    @NSManaged nonisolated public var price: Double
    @NSManaged nonisolated public var done: Bool
    @NSManaged nonisolated public var blob: Data?
    @NSManaged nonisolated public var identifier: UUID?
    @NSManaged nonisolated public var tags: NSSet?

}

// MARK: Generated accessors for tags
extension Item {

    @objc(addTagsObject:)
    @NSManaged nonisolated public func addToTags(_ value: Tag)

    @objc(removeTagsObject:)
    @NSManaged nonisolated public func removeFromTags(_ value: Tag)

    @objc(addTags:)
    @NSManaged nonisolated public func addToTags(_ values: NSSet)

    @objc(removeTags:)
    @NSManaged nonisolated public func removeFromTags(_ values: NSSet)

}

extension Item : Identifiable {

}
