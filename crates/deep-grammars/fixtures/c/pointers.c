static int buf[64];
static char name[32] = "agent";
static int grid[4][4];

int (*handler)(int);

int apply(int (*fn)(int), int value) {
    return fn(value);
}

int twice(int value) {
    return value * 2;
}

void fill(int *dest, int count, int value) {
    int i;
    for (i = 0; i < count; i++) {
        dest[i] = value;
    }
}

int main(void) {
    int matrix[3][3];
    matrix[0][0] = 1;
    grid[1][2] = matrix[0][0];
    fill(buf, 64, 7);
    name[0] = 'a';
    handler = twice;
    return apply(handler, buf[0]);
}
