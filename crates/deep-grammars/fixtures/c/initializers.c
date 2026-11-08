struct Config {
    int size;
    unsigned flags;
    char *name;
};

static const struct Config defaults = {16, 0u, "default"};

_Static_assert(sizeof(int) >= 2, "int too small");

static char letters[4] = {'a', 'b', 'c', '\0'};

static char *joined = "left" "-" "right";

static const struct Config overrides = {
    .size = 32,
    .flags = 1u,
    .name = "override",
};

int total(int first, int second) {
    int values[3] = {first, second, first + second};
    return values[0] + values[1] + values[2];
}
