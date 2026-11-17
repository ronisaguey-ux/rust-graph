// `@interface Foo ()` / `@implementation Foo ()`: a class extension is the
// same header with an empty parenthesised list where a category spells a name.
#import <Foundation/Foundation.h>

@interface Widget ()
@property (nonatomic, assign, getter=isReady) BOOL ready;
- (void)refresh;
@end

@interface Widget : NSObject <NSCopying>
- (void)refresh;
@end

@implementation Widget ()
- (void)refresh
{
    _ready = YES;
}
@end

@implementation Widget
- (void)refresh
{
    _ready = NO;
}
@end
