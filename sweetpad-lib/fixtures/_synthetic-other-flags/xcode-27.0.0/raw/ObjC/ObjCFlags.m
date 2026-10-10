#import <Foundation/Foundation.h>

#if !defined(SWEETPAD_OTHER_C)
#error "OTHER_CFLAGS did not reach the ObjC compile"
#endif

NSString *ObjCFlagsName(void) { return @"objc"; }
