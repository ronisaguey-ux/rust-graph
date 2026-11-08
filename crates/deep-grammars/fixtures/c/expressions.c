unsigned long mix(unsigned long state) {
    state ^= state >> 13;
    state *= 2654435761u;
    state ^= state << 7;
    return state;
}

int budget(int a, int b, int c) {
    int v = a + b * c - a / 2 % 3;
    int w = (a + b) * (c - 1);
    int shifted = (a << 2) | (b >> 1);
    int masked = shifted & 0xff ^ ~a;
    int flag = a > b && b != c || !a;
    int pick = a > b ? a : b;
    int cast = (int)3.75 + sizeof(int) + sizeof a;
    int negative = -a + +b;
    return v + w + masked + flag + pick + cast + negative;
}
