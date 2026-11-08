#import <Foundation/Foundation.h>
#import "Greeter.h"

@implementation Greeter

- (instancetype)initWithName:(NSString *)name {
    self = [super init];
    if (self != nil) {
        _name = name;
    }
    return self;
}

- (void)greet {
    NSLog(@"hello %@", _name);
}

- (NSString *)name {
    return _name;
}

@end

int main(int argc, const char *argv[]) {
    Greeter *greeter = [[Greeter alloc] initWithName:@"world"];
    [greeter greet];
    return 0;
}
