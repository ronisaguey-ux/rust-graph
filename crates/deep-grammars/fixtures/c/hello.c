#include <stdio.h>
#include <stdlib.h>
#include "local.h"

static int counter = 0;

int add(int a, int b) {
    return a + b;
}

int main(void) {
    int total = add(2, 3);
    printf("total = %d\n", total);
    return 0;
}
