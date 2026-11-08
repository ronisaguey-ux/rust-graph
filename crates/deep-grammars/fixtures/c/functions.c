int fib(int n);

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

char *label(int code) {
    return code == 0 ? "ok" : "error";
}
