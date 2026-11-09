// A type assertion inside a substitution is an ordinary expression: `as`
// binds its receiver and the call on it is a call expression, so the CST
// names `(x as Foo).bar()` instead of hiding it in one literal. A template
// inside a substitution is a template of its own, here and in a `.js` file.
export const label = `${(x as Foo).bar()}`;
export const nested = `${`${count as number}`.length}`;
