/* An array parameter may qualify its size: `[restrict]`, `[static 4]` and
 * `[const 8]`, which is where the size would stand. */
int walk(char *const names[restrict], int count[static 4], long sizes[const 8]) {
    return count[0] + (int)sizes[1] + (names[0] != 0);
}
