/* A declaration whose later declarators are pointers: the first names the
   declaration, the rest follow a comma. */
typedef struct handle handle_t;

handle_t *handles(int count);

void use(void)
{
	handle_t slot, *open_slot = slot;
	(void)open_slot;
	(void)handles(1);
}
