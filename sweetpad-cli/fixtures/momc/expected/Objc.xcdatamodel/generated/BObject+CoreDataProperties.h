//
//  BObject+CoreDataProperties.h
//  
//
//  Created by $(AUTHOR) on $(DATE).
//
//  This file was automatically generated and should not be edited.
//

#import "BObject.h"


NS_ASSUME_NONNULL_BEGIN

@interface BObject (CoreDataProperties)

+ (NSFetchRequest<BObject *> *)fetchRequest;

@property (nullable, nonatomic, copy) NSString *title;
@property (nullable, nonatomic, retain) NSSet<A *> *as;

@end

@interface BObject (CoreDataGeneratedAccessors)

- (void)addAsObject:(A *)value;
- (void)removeAsObject:(A *)value;
- (void)addAs:(NSSet<A *> *)values;
- (void)removeAs:(NSSet<A *> *)values;

@end

NS_ASSUME_NONNULL_END
