#import <Foundation/Foundation.h>

static NSString *kName = @"default";

NSString *describe(int value) {
    if (value == 0) {
        return @"zero";
    }
    return @"nonzero";
}

int literals(void) {
    NSString *text = @"plain";
    char first = 'a';
    char newline = '\n';
    unsigned long big = 1000000UL;
    double fraction = 2.5e-3;
    int hexed = 0xFF;
    NSString *joined = @"left" @"right";
    return (int)sizeof(text) + first + newline + hexed + (int)big + (int)fraction;
}

NSString *selector_name(void) {
    SEL pick = @selector(describe:);
    const char *encoding = @encode(int);
    return NSStringFromSelector(pick);
}
