/* A declarator that is a parenthesized function pointer, with the macro that
   marks where the declaration ends. The parentheses are what makes the
   trailing macro unambiguous: a plain name there would be the name. */
void (*log_emit)(struct emitter *, const char *) NO_RETURN_ATTR;
key_t *(*key_open)(struct ctx *, const char *) PLUGIN_ATTR;
