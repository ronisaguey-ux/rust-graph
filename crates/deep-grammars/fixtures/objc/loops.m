#import <Foundation/Foundation.h>

int sum(id items) {
    int total = 0;
    for (int index = 0; index < 10; index++) {
        total += index;
        if (total > 20) {
            break;
        }
    }
    return total;
}

int count_objects(NSArray *items) {
    int found = 0;
    for (id item in items) {
        if (item != nil) {
            found = found + 1;
        }
    }
    return found;
}

int classify(int code) {
    switch (code) {
    case 0:
        return 10;
    case 1:
        return 20;
    default:
        return 30;
    }
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

int countdown(int n) {
    int total = 0;
    while (n > 0) {
        total += n;
        n--;
    }
    return total;
}
