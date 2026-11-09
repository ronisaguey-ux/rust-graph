/* Return types, parameters, locals and casts that are typedef names. */
namespace app {

static Result *build(Context &context, int flags);

extern Config *current;

Result *build(Context &context, int flags) {
    Result *result = make_result(flags);
    LockGuard lock(context.mutex);
    Node *root = (Node *) result->root;
    return root == nullptr ? result : result;
}

}
