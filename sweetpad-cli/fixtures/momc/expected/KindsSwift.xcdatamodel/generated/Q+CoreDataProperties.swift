//
//  Q+CoreDataProperties.swift
//  
//
//  Created by $(AUTHOR) on $(DATE).
//
//  This file was automatically generated and should not be edited.
//

public import Foundation
public import CoreData


public typealias QCoreDataPropertiesSet = NSSet

extension Q {

    @nonobjc public class func fetchRequest() -> NSFetchRequest<Q> {
        return NSFetchRequest<Q>(entityName: "Q")
    }

    @NSManaged nonisolated public var list: P?

}
