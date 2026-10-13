//
//  R+CoreDataProperties.swift
//  
//
//  Created by $(AUTHOR) on $(DATE).
//
//  This file was automatically generated and should not be edited.
//

public import Foundation
public import CoreData


public typealias RCoreDataPropertiesSet = NSSet

extension R {

    @nonobjc public class func fetchRequest() -> NSFetchRequest<R> {
        return NSFetchRequest<R>(entityName: "R")
    }

    @NSManaged nonisolated public var owner: P?

}
