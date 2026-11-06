//
//  P+CoreDataProperties.h
//  
//
//  Created by $(AUTHOR) on $(DATE).
//
//  This file was automatically generated and should not be edited.
//

#import "P+CoreDataClass.h"


NS_ASSUME_NONNULL_BEGIN

@interface P (CoreDataProperties)

+ (NSFetchRequest<P *> *)fetchRequest;

@property (nullable, nonatomic, retain) NSData *bin;
@property (nullable, nonatomic, copy) NSUUID *uid;
@property (nullable, nonatomic, copy) NSDate *day;
@property (nonatomic) BOOL yes;
@property (nonatomic) double d;
@property (nonatomic) int32_t i;
@property (nonatomic) int64_t l;
@property (nullable, nonatomic, copy) NSString *s;
@property (nonatomic) int32_t plain;
@property (nullable, nonatomic, retain) Q *q;
@property (nullable, nonatomic, retain) NSSet<R *> *rs;
@property (nullable, nonatomic, retain) NSOrderedSet<Q *> *qs;

@end

@interface P (CoreDataGeneratedAccessors)

- (void)addRsObject:(R *)value;
- (void)removeRsObject:(R *)value;
- (void)addRs:(NSSet<R *> *)values;
- (void)removeRs:(NSSet<R *> *)values;

- (void)insertObject:(Q *)value inQsAtIndex:(NSUInteger)idx;
- (void)removeObjectFromQsAtIndex:(NSUInteger)idx;
- (void)insertQs:(NSArray<Q *> *)value atIndexes:(NSIndexSet *)indexes;
- (void)removeQsAtIndexes:(NSIndexSet *)indexes;
- (void)replaceObjectInQsAtIndex:(NSUInteger)idx withObject:(Q *)value;
- (void)replaceQsAtIndexes:(NSIndexSet *)indexes withQs:(NSArray<Q *> *)values;
- (void)addQsObject:(Q *)value;
- (void)removeQsObject:(Q *)value;
- (void)addQs:(NSOrderedSet<Q *> *)values;
- (void)removeQs:(NSOrderedSet<Q *> *)values;

@end

NS_ASSUME_NONNULL_END
