// This target is in no scheme, so only a `-target` build prepares it.
#import "DeepThing.h"

@interface OrphanThing : NSObject
@end

@implementation OrphanThing
+ (NSString *)name { return [DeepThing deepName]; }
@end
