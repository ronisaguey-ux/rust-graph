/* GNU attributes, thread_local storage and inline assembly in C++. */
namespace app {

__attribute__((always_inline)) inline int twice(int value) {
    return value + value;
}

thread_local int depth = 0;

int read_counter(void) {
    int counter;
    __asm__ volatile("mov %0, %%eax" : "=r"(counter));
    return counter;
}

}
