#import <Foundation/Foundation.h>

typedef struct Range {
    int start;
    int length;
} Range;

enum Mode {
    MODE_IDLE,
    MODE_BUSY = 2,
};

int mix(int state) {
    state ^= state >> 13;
    state *= 2654435761u;
    state ^= state << 7;
    return state;
}

int budget(int a, int b) {
    int value = a + b * 3 - a / 2 % 5;
    int shifted = (a << 2) | (b >> 1);
    int masked = shifted & 0xff ^ ~a;
    int flag = a > b && b != 0 || !a;
    int pick = a > b ? a : b;
    int cast = (int)4.5 + sizeof(int) + sizeof a;
    return value + masked + flag + pick + cast;
}

int range_length(Range range) {
    Range local = range;
    return local.length;
}

BOOL is_busy(int mode) {
    return mode == MODE_BUSY ? YES : NO;
}
