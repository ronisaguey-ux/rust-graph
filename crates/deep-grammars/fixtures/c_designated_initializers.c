/* Designated initializers and the comma operator in a statement. */
struct point {
    int x;
    int y;
};

struct point origin = { .x = 0, .y = 0 };

int table[4] = { [1] = 7, [3] = 9 };

int pair(int input) {
    int left = 0, right = 0;
    left = input, right = input + 1;
    return left + right;
}
