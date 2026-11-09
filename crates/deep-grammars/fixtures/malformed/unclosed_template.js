// A substitution with no expression, and a template that cannot close: the
// braces and the backtick are reported where they are, and the statements
// around them keep their place.
const empty = `${}`;
const broken = `${a
