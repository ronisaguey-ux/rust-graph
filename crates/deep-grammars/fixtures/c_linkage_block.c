/* A header that wraps its declarations in a C++ linkage block. */
#ifdef __cplusplus
extern "C" {
#endif

int widget_open(const char *name);
int widget_close(int handle);

#ifdef __cplusplus
}
#endif
