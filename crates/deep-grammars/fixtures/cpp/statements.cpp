#include <stdexcept>
#include <string>
#include <vector>

namespace math {

enum class Rounding {
    Down,
    Nearest,
    Up,
};

long mix(long state) {
    state ^= state >> 13;
    state *= 2654435761L;
    state ^= state << 7;
    return state;
}

int clamp_value(int value, int low, int high) {
    if (value < low) {
        return low;
    } else if (value > high) {
        return high;
    }
    return value;
}

int classify(int n) {
    switch (n) {
    case 0:
        return 0;
    case 1:
        return 1;
    default:
        break;
    }
    return n;
}

int sum(const std::vector<int> &values) {
    int total = 0;
    for (int value : values) {
        total += value;
    }
    return total;
}

int guarded(int divisor) {
    try {
        if (divisor == 0) {
            throw std::runtime_error("zero");
        }
        return 100 / divisor;
    } catch (const std::exception &error) {
        return -1;
    }
}

}  // namespace math
