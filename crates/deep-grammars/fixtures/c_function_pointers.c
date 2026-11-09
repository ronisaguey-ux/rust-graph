/* Function pointers: a typedef, a struct field, a parameter and a cast. */
typedef int (*handler)(void *context, int value);
typedef outcome_t (*outcome_handler)(int value);

typedef struct _table {
    handler dispatch;
    void (*reset)(void);
} table;

int invoke(handler call, void *context) {
    return call(context, 1);
}

int install(table *into) {
    into->dispatch = (handler) invoke;
    return into->dispatch != 0;
}
