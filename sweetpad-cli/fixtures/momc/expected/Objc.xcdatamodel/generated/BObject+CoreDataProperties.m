//
//  BObject+CoreDataProperties.m
//  
//
//  Created by $(AUTHOR) on $(DATE).
//
//  This file was automatically generated and should not be edited.
//

#import "BObject+CoreDataProperties.h"

@implementation BObject (CoreDataProperties)

+ (NSFetchRequest<BObject *> *)fetchRequest {
	return [NSFetchRequest fetchRequestWithEntityName:@"B"];
}

@dynamic title;
@dynamic as;

@end
