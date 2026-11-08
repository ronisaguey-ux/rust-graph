#import <Foundation/Foundation.h>

static int table[16];

int gcd(int a, int b) {
    while (b != 0) {
        int t = b;
        b = a % b;
        a = t;
    }
    return a;
}

int fib(int n) {
    if (n < 2) {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}

double average(const double *values, int count) {
    double total = 0.0;
    int i;
    for (i = 0; i < count; i++) {
        total += values[i];
    }
    return total / count;
}

int fill(int *dest, int count, int value) {
    int i;
    for (i = 0; i < count; i++) {
        dest[i] = value;
    }
    return count;
}

id dispatch(id target, int code) {
    if ([target respondsToSelector:@selector(handle:)]) {
        [target handle:code];
    }
    return target;
}
