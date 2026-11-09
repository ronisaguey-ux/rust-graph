// Tags: a member tag, a namespaced tag, a nested pair, a self-closing child
// and a fragment. Only the braces are code; tag names and text are tokens.
const member = <A.B/>;
const deep = <A.B.C>x</A.B.C>;
const namespaced = <a:b>text</a:b>;
const nested = <div><span/>{f(y)}</div>;
const fragment = <>before{g(x)}after</>;
const pair = <ul>{items.map(item => <li key={item.id}>{item.name}</li>)}</ul>;
