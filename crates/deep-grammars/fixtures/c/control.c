int classify(int n) {
    if (n < 0) {
        return -1;
    } else if (n == 0) {
        return 0;
    } else {
        return 1;
    }
}

int sum_to(int n) {
    int sum = 0;
    int i;
    for (i = 0; i < n; i++) {
        sum += i;
    }
    return sum;
}

int count_down(int n) {
    int steps = 0;
    while (n > 0) {
        n--;
        steps++;
    }
    return steps;
}

int retry(int limit) {
    int tries = 0;
    do {
        tries++;
        if (tries == limit) {
            goto done;
        }
    } while (tries < limit);
done:
    return tries;
}

int loop_forever(void) {
    int n = 0;
    for (;;) {
        n++;
        if (n > 10) {
            break;
        }
        continue;
    }
    return n;
}
