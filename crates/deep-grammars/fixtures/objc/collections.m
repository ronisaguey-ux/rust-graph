#import <Foundation/Foundation.h>

int collect(id source) {
    NSMutableArray *items = [NSMutableArray array];
    NSDictionary *mapping = [NSDictionary dictionary];
    [items addObject:[source description]];
    [items addObject:mapping];
    items[0] = @"replaced";
    id first = [items objectAtIndex:0];
    id second = items[1];
    id value = [mapping objectForKey:@"key"];
    return (int)[items count] + (int)[mapping count] + (first == second) + (value == nil);
}
