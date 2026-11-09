/* GCC and MSVC attributes, before and after the declarator. */
__attribute__((noreturn)) void die(const char *why);

__attribute__((always_inline)) static inline int twice(int value) {
    return value + value;
}

__declspec(dllexport) int exported(void);

int aligned_value __attribute__((aligned(4))) = 0;
