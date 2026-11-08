#import <Foundation/Foundation.h>

@class Store;

@protocol Reporter <NSObject>
@required
- (void)report:(NSString *)message;
- (int)count;
@optional
- (void)reset;
@end

@interface Worker : NSObject <Reporter> {
    id _target;
    int _state;
    NSString *_label;
}

@property (nonatomic, strong) NSString *label;
@property (nonatomic, assign) int state;

- (instancetype)initWithTarget:(id)target;
- (void)start;
- (int)count;

@end

@implementation Worker

@synthesize label = _label;
@synthesize state = _state;

- (instancetype)initWithTarget:(id)target {
    self = [super init];
    if (self) {
        _target = target;
    }
    return self;
}

- (void)start {
    _state = 1;
}

- (void)report:(NSString *)message {
    NSLog(@"%@", message);
}

- (int)count {
    return _state;
}

@end
