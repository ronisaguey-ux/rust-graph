// A key spelled like one of this grammar's keywords is a name, not layout:
// a pair whose key the layout filter drops reads m:[0] here and
// m:[m:[n:type,0]] in the lexer, which is one whole file of disagreement.
const schema = { type: "boolean", default: false, class: "solid" };

const copy = { type: schema.type, default: schema.default };

export default copy;
