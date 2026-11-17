/* A struct member's own array suffix: the type word and the member's name are
   two names in a row, and the suffix belongs to the member. */
struct node {
	int slot[4];
	char label[8];
};

int first(struct node *node)
{
	return node->slot[0];
}
