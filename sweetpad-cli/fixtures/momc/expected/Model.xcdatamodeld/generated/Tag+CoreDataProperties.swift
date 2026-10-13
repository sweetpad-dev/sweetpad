//
//  Tag+CoreDataProperties.swift
//  
//
//  Created by $(AUTHOR) on $(DATE).
//
//  This file was automatically generated and should not be edited.
//

public import Foundation
public import CoreData


public typealias TagCoreDataPropertiesSet = NSSet

extension Tag {

    @nonobjc public class func fetchRequest() -> NSFetchRequest<Tag> {
        return NSFetchRequest<Tag>(entityName: "Tag")
    }

    @NSManaged nonisolated public var title: String?
    @NSManaged nonisolated public var items: NSSet?

}

// MARK: Generated accessors for items
extension Tag {

    @objc(addItemsObject:)
    @NSManaged nonisolated public func addToItems(_ value: Item)

    @objc(removeItemsObject:)
    @NSManaged nonisolated public func removeFromItems(_ value: Item)

    @objc(addItems:)
    @NSManaged nonisolated public func addToItems(_ values: NSSet)

    @objc(removeItems:)
    @NSManaged nonisolated public func removeFromItems(_ values: NSSet)

}

extension Tag : Identifiable {

}
