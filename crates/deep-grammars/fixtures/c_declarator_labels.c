/* A declarator that names the machine register it lives in: the label sits
 * where an attribute would, in front of the initializer. */
long move(long fd) {
    register long r4 __asm__("$4") = fd;
    register long r7 __asm__("$7");
    return r4 + r7;
}
