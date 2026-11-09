// The `${...}` of a template literal is ordinary code: the call written in it
// is a call expression, a template inside a substitution is a template of its
// own, a tag before a literal keeps the literal as the call's argument, and a
// literal with no substitution is still one template.
const message = `hi ${format(user)} and ${user.name}`;
const nested = `${`inner ${g(y)}`}`;
const tagged = tag`x ${h(z)}`;
const plain = `no substitutions`;
const braced = `${ { a: 1 }.a }`;
function use(user, y, z, tag, g, h, format) {
  return [message, nested, tagged, plain, braced];
}
