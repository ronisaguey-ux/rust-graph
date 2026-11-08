#include <cstdio>
#include <string>

#define SQUARE(x) ((x) * (x))
#define TRACE(msg) printf("%s\n", msg)

#if defined(_DEBUG)
#define CHECK(c) do { if (!(c)) { TRACE("failed"); } } while (0)
#else
#define CHECK(c) ((void)0)
#endif

#define MULTI(a, b) \
    do {            \
        (a) += (b); \
        (b) *= 2;   \
    } while (0)

namespace logging {

static int verbosity = 1;

const char *level(int code) {
    switch (code) {
    case 0:
        return "info";
    case 1:
        return "warn";
    default:
        return "error";
    }
}

int emit(const char *message, int code) {
    CHECK(message != 0);
    TRACE(message);
    MULTI(verbosity, code);
    printf("%s: %s (%d)\n", level(code), message, verbosity);
    return verbosity;
}

}  // namespace logging

int main() {
    int n = 3;
    int squared = SQUARE(n + 1);
    return logging::emit("ready", squared);
}
