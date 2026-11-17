/* A designated initializer may nest its designators: a member of a member, and
 * an element of a member's array. */
struct inner {
    long sec;
    long usec;
    long opts[2];
};

struct outer {
    struct inner value;
};

struct outer timer = { .value.sec = 1, .value.usec = 2, .value.opts[1] = 3 };
