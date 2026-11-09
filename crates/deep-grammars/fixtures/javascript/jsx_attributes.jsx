// An attribute value in braces is ordinary code: a call written in one is a
// call_expression, a spread is a spread of an expression, and a quoted value is
// a string with its entities left alone.
function panel(user, x, id, props) {
  return <button onClick={() => handle(x)} title="press &amp; go" data-id={id(x)} {...props}>press</button>;
}
function link(user) {
  return <a href="/u" aria-label={label(user)}>profile</a>;
}
