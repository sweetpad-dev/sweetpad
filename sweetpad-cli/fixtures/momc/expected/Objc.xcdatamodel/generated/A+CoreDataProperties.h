//
//  A+CoreDataProperties.h
//  
//
//  Created by $(AUTHOR) on $(DATE).
//
//  This file was automatically generated and should not be edited.
//

#import "A+CoreDataClass.h"


NS_ASSUME_NONNULL_BEGIN

@interface A (CoreDataProperties)

+ (NSFetchRequest<A *> *)fetchRequest NS_SWIFT_NAME(fetchRequest());

@property (nonatomic) int16_t i16;
@property (nullable, nonatomic, copy) NSNumber *i32;
@property (nullable, nonatomic, copy) NSNumber *dbl;
@property (nonatomic) float flt;
@property (nullable, nonatomic, copy) NSNumber *flag;
@property (nullable, nonatomic, copy) NSDecimalNumber *dec;
@property (nonatomic) NSTimeInterval when;
@property (nullable, nonatomic, copy) NSURL *link;
@property (nullable, nonatomic, retain) [String] *list;
@property (nullable, nonatomic, retain) NSObject *obj;
@property (nullable, nonatomic, retain) NSObject *oid;
@property (nullable, nonatomic, copy) NSString *default;
@property (nullable, nonatomic, copy) NSString *tmp;
@property (nullable, nonatomic, retain) BObject *b;
@property (nullable, nonatomic, retain) NSOrderedSet<C *> *cs;

@end

@interface A (CoreDataGeneratedAccessors)

- (void)insertObject:(C *)value inCsAtIndex:(NSUInteger)idx;
- (void)removeObjectFromCsAtIndex:(NSUInteger)idx;
- (void)insertCs:(NSArray<C *> *)value atIndexes:(NSIndexSet *)indexes;
- (void)removeCsAtIndexes:(NSIndexSet *)indexes;
- (void)replaceObjectInCsAtIndex:(NSUInteger)idx withObject:(C *)value;
- (void)replaceCsAtIndexes:(NSIndexSet *)indexes withCs:(NSArray<C *> *)values;
- (void)addCsObject:(C *)value;
- (void)removeCsObject:(C *)value;
- (void)addCs:(NSOrderedSet<C *> *)values;
- (void)removeCs:(NSOrderedSet<C *> *)values;

@end

NS_ASSUME_NONNULL_END
