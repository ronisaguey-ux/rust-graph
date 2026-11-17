/* C99 compound literals: a type and a brace list, in value and in argument
 * position, with a keyword type and with a typedef-shaped one. */
struct pair {
    long s;
    long n;
};

long sum(struct pair *p) {
    return p->s + p->n;
}

long read(int s, long ns) {
    struct pair *p = (struct pair){ s, ns };
    long first = (long []){ s, ns }[0];
    return sum(&(struct pair){ .s = s, .n = ns }) + first + p->s;
}
