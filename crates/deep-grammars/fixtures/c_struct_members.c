/* A struct member whose declarator is an array: a sized one beside a plain
 * member, a flexible one at the end, and an anonymous-free list. */
struct fdop {
    struct fdop *next, *prev;
    int cmd, fd;
    char path[];
};

struct args {
    int p[2];
    long count;
};

int fd_of(struct fdop *op) {
    return op->cmd + op->path[0] + op->fd;
}

int pair_of(struct args *a) {
    return a->p[1];
}
