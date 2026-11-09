/* A visibility macro in front of a declaration, one or two words deep. */
#define EXPORT __attribute__((visibility("default")))

EXPORT int public_entry(int argc, char **argv);
EXPORT const char *public_name(void);

static EXPORT int hidden_entry(void) {
    return 0;
}

int main(void) {
    return hidden_entry();
}
