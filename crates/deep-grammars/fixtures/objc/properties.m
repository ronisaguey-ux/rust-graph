#import <Foundation/Foundation.h>

@protocol Drawable <NSObject>
- (void)draw;
@end

@interface Canvas : NSObject {
    id _delegate;
}

@property (nonatomic, weak) id delegate;
@property (nonatomic, readonly) int width;
@property (nonatomic, copy) NSString *title;

- (void)addShape:(id)shape atIndex:(int)index;
- (id)shapeAtIndex:(int)index;

@end

@implementation Canvas

@dynamic delegate;

- (void)addShape:(id)shape atIndex:(int)index {
    NSLog(@"add %@ at %d", shape, index);
}

- (id)shapeAtIndex:(int)index {
    return nil;
}

- (void)draw {
    [self draw];
}

@end
