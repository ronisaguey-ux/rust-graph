/* A field macro that expands to whole members, which carries no declarator
   and no semicolon of its own. */
struct _object {
    HEADER_FIELDS
    holder_t **names;
    int count;
};

struct _object *object_new(void);
