#import <Foundation/Foundation.h>

int guarded(int divisor) {
    @try {
        if (divisor == 0) {
            @throw [NSException exceptionWithName:@"Zero" reason:@"divide" userInfo:nil];
        }
        return 100 / divisor;
    } @catch (NSException *error) {
        NSLog(@"caught %@", error);
        return -1;
    } @finally {
        NSLog(@"done");
    }
}

int pooled(void) {
    @autoreleasepool {
        NSMutableArray *items = [NSMutableArray array];
        [items addObject:@"one"];
        [items addObject:@"two"];
        return (int)[items count];
    }
}

void locked(id lock) {
    @synchronized(lock) {
        NSLog(@"inside");
    }
}

@interface Cache : NSObject
@property (nonatomic, strong) NSMutableDictionary *entries;
- (id)objectForKey:(NSString *)key;
@end

@implementation Cache

- (id)objectForKey:(NSString *)key {
    return [_entries objectForKey:key];
}

@end
