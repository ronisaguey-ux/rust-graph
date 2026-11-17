/* Macros whose second argument names a type: va_arg and offsetof. A type name
 * always starts with a declaration specifier, so no value is read as one. */
struct pair {
    long s;
    long n;
};

const char *next_text(__builtin_va_list ap) {
    return va_arg(ap, const char *);
}

long tail_of(void) {
    return offsetof(struct pair, n);
}
