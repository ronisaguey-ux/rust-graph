// All three stacks at once: a template whose substitution holds a JSX element
// whose attribute holds a template whose substitution holds a call.
const mixed = `x ${<div a={`inner ${f(y)}`}>{<span/>}</div>} z`;
const plain = <div className="box">text</div>;
