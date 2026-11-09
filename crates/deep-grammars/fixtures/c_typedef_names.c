/* Declarations whose type is a name the grammar cannot know. */
typedef struct _buffer {
    int size;
} buffer;

buffer *buffer_new(int size);
static const buffer *buffer_empty(void);

int fill(void) {
    buffer *slot;
    const buffer *limit;
    return slot == limit;
}
