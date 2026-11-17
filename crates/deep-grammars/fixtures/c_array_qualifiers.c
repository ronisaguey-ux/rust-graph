/* C99 array qualifiers: a parameter's brackets may hold a type qualifier or
   `static`, with or without a size. */
static void fill(int n, char buffer[restrict]);

void use(int n, char *src)
{
	char out[8];
	fill(n, out);
	(void)src;
}
