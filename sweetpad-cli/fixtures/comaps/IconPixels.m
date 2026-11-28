// The home screen asks CoreUI for an icon by idiom, size and appearance.
// Exercise the same catalog lookup on macOS without changing a simulator's home screen.
#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>
#import <CommonCrypto/CommonDigest.h>
#import <dlfcn.h>

@interface NSObject (CatalogProbe)
- (instancetype)initWithURL:(NSURL *)url error:(NSError **)error;
- (id)iconImageWithName:(NSString *)name scaleFactor:(CGFloat)scale deviceIdiom:(NSInteger)idiom
          deviceSubtype:(NSUInteger)subtype displayGamut:(NSInteger)gamut layoutDirection:(NSInteger)direction
    sizeClassHorizontal:(NSInteger)horizontal sizeClassVertical:(NSInteger)vertical
            desiredSize:(CGSize)size appearanceName:(NSString *)appearance;
- (CGImageRef)image;
@end

int main(int argc, char **argv) {
    @autoreleasepool {
        if (argc != 3 || !dlopen("/System/Library/PrivateFrameworks/CoreUI.framework/CoreUI", RTLD_LAZY)) return 1;
        NSError *error = nil;
        id catalog = [[NSClassFromString(@"CUICatalog") alloc] initWithURL:[NSURL fileURLWithPath:@(argv[1])] error:&error];
        if (!catalog) { NSLog(@"Could not open catalog: %@", error); return 1; }
        for (NSInteger idiom = 1; idiom <= 2; idiom++) {
            for (NSString *appearance in @[@"UIAppearanceAny", @"UIAppearanceDark", @"ISAppearanceTintable"]) {
                id icon = [catalog iconImageWithName:@(argv[2]) scaleFactor:1 deviceIdiom:idiom deviceSubtype:0
                                        displayGamut:0 layoutDirection:0 sizeClassHorizontal:0 sizeClassVertical:0
                                         desiredSize:CGSizeMake(1024, 1024) appearanceName:appearance];
                CGImageRef image = [icon image];
                if (!image || CGImageGetWidth(image) != 1024 || CGImageGetHeight(image) != 1024) {
                    NSLog(@"Missing 1024px %@ for idiom %ld, %@", @(argv[2]), (long)idiom, appearance);
                    return 1;
                }
                NSMutableData *pixels = [NSMutableData dataWithLength:1024 * 1024 * 4];
                CGColorSpaceRef rgb = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
                CGContextRef context = CGBitmapContextCreate(pixels.mutableBytes, 1024, 1024, 8, 4096, rgb,
                                                           (CGBitmapInfo)kCGImageAlphaPremultipliedLast);
                CGContextDrawImage(context, CGRectMake(0, 0, 1024, 1024), image);
                CGContextRelease(context);
                CGColorSpaceRelease(rgb);
                unsigned char digest[CC_SHA256_DIGEST_LENGTH];
                CC_SHA256(pixels.bytes, (CC_LONG)pixels.length, digest);
                printf("%ld %s ", (long)idiom, appearance.UTF8String);
                for (size_t i = 0; i < sizeof(digest); i++) printf("%02x", digest[i]);
                puts("");
            }
        }
    }
}
