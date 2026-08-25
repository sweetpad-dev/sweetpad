#import "Widget.h"
// A sibling directory's header, named by no HEADER_SEARCH_PATHS entry: only the
// project header map resolves it.
#import "DeepThing.h"
// This target's Swift half, which exists only once a build has generated it.
#import "HeaderMaps-Swift.h"
// Another target's public header, reached through the framework it installs.
#import <HeaderMapsCore/CoreThing.h>

@implementation Widget
+ (NSString *)describe {
  return [NSString stringWithFormat:@"%@ %@ %@", [DeepThing deepName],
                                    [CoreThing coreName],
                                    [[Greeter new] greeting]];
}
@end
