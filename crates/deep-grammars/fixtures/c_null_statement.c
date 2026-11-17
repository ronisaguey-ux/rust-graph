/* The null statement: a body that is nothing, after a loop header and after a
 * label. */
int read_one(void);

int drain(int *out) {
    int r;
    while ((r = read_one()) == -1);
    for (int i = 0; i < 4; i++);
    switch (r) {
    case 0: ;
    default: break;
    }
    return r;
}
