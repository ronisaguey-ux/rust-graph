#define MAX(a, b) ((a) > (b) ? (a) : (b))
#define SQUARE(x) ((x) * (x))
#define LOG(fmt, ...) printf(fmt, __VA_ARGS__)

#define LONG_MACRO(x) \
    do {              \
        (x) += 1;     \
    } while (0)

#if defined(DEBUG)
static int verbose = 1;
#elif defined(RELEASE)
static int verbose = 0;
#else
static int verbose = -1;
#endif

int clamp(int value, int low, int high) {
    int result = value;
    if (result > high) {
        result = high;
    }
    if (result < low) {
        result = low;
    }
    return result;
}

int main(void) {
    int a = 3;
    int b = 7;
    int best = MAX(a, b);
    int sq = SQUARE(best);
    LOG("best=%d sq=%d verbose=%d\n", best, sq, verbose);
    LONG_MACRO(a);
    return clamp(a, 0, 100);
}
