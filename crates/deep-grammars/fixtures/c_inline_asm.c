/* Inline assembly: a template, the output, input and clobber sections. */
int stack_pointer(void) {
    int *sp;
    __asm__ volatile("mov %0, sp" : "=r"(sp));
    __asm__ volatile("" : : : "memory");
    return sp != 0;
}
