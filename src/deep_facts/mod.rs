//! Deep facts: the same per-file record the one-pass lexer writes, read off the
//! CST instead of off a token stream.
//!
//! ```text
//! &[u8] + Cst -> walk -> Facts { tag, lines, imports, symbols, vars }
//! ```
//!
//! The record is `crate::facts::Facts`, the one `pipeline` caches and `resolve`
//! reads; nothing here invents a second schema. The lexer path (`crate::parser`)
//! talks in statements and guesses where a block ends; this path knows the tree,
//! so nesting, qualified names and call arguments are facts, not inference.
//!
//! What is a definition, a call or an import is per language and written down as
//! data (`PY_DEFS`, `JS_DEFS`, `RS_DEFS`, `GO_DEFS`, `JAVA_DEFS`, ...), selected
//! by [`kinds`]; the walk itself is language neutral. Python, JavaScript,
//! TypeScript, TSX, Rust, Go, Java, C, C++, C#, Kotlin, Scala and Swift have
//! tables, and Rust's and the C family's walk arms live in their own regions
//! below. The C-family languages share one walk and one reader: what differs
//! between C, C++, Java, C#, Go, Kotlin, Scala and Swift is the node table
//! (`C_DEFS`, `CPP_DEFS`, `JAVA_DEFS`, `CS_DEFS`, `GO_DEFS`, `KT_DEFS`,
//! `SC_DEFS`, `SW_DEFS` and its call/import list), never a copy of the reader.
//! JSON has empty tables
//! on purpose: a data file's only fact is the module symbol, so [`lang_for`]
//! answers `None` for it and the lexer path is kept. C and C++ also answer
//! `None` from `lang_for`: their tables feed `tests/facts_c_cpp.rs` and the
//! `--compare` audit, but the differential gate does not yet prefer the CST
//! record for enough real files. C#, Kotlin, Scala and Swift are force-only for
//! the same reason, and they answer `--deep` so the C-family claim is testable;
//! the other grammars fall back to the module symbol until their table is
//! written.
//!
//! Requires the `deep` and `parser` features: the Cst comes from
//! `deep_integration`, which names the runtime's kind table, and the runtime
//! (`src/deep`) is what the `parser` feature compiles.

mod c_family;
mod go;
mod java;
mod rust;

use crate::deep::syntax::Language;
use crate::deep_integration::{Cst, Lang, Node, NONE};
use crate::facts::{Call, Facts, Import, Symbol, IMPORT_MODULE, IMPORT_STAR, KIND_CLASS, KIND_FUNCTION, KIND_MODULE};
use crate::json::push_string;

// ---------------------------------------------------------------------------
// The per-language node kinds.
// ---------------------------------------------------------------------------

/// How a qualified name is built for each construct: the name of a definition,
/// the receiver of the dot, and the table of nodes that mean something.
///
/// The kinds are the grammar's own names (`Language::name`), so a grammar that
/// renames a rule breaks this table loudly in the differential test rather than
/// silently emitting nothing.
struct Kinds {
    definitions: &'static [Def],
    calls: &'static [&'static str],
    imports: &'static [&'static str],
    /// Tokens the lexer spells as keywords whatever the context, but which are
    /// names where a name is read. Python's soft keywords are context free in
    /// this grammar, so `match.group(1)` carries the `match` kind, not
    /// `identifier`; the mapping reads the leaf's text as the name it is.
    soft: &'static [&'static str],
    /// Which family's declaration shape the grammar writes. They differ in
    /// where a name sits and how a statement is spelled, and nothing else.
    family: Family,
}

/// How a grammar spells its statements. `Plain` is every language whose
/// declarations this file has no table for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Plain,
    Js,
    Rust,
    Go,
    Java,
}

/// One definition node kind: where its name sits, where its parameters sit and
/// whether it owns a body (a class or a function).
struct Def {
    kind: &'static str,
    /// The children that may carry the name, in order. Empty means the name
    /// comes from the parent (a function expression bound to a variable).
    name: &'static [&'static str],
    /// The child holding the parameter list, if the header has one.
    params: Option<&'static str>,
    /// Which occurrence of `params` is the list: Go puts a method's receiver
    /// first (`func (c *Cache) Get(key K)`), so its own list is the second.
    params_nth: u8,
    /// The name is the last child of a `name` kind rather than the first. Java
    /// writes the return type first: `Item find(int id)`.
    name_last: bool,
    /// The owner is the receiver's type, read off the first parameter list
    /// (Go's method receiver).
    receiver: bool,
    /// `true` for a container whose members take its name as an owner.
    container: bool,
}

const PY_DEFS: &[Def] = &[
    Def { kind: "class_definition", name: &["identifier", "match", "case", "type"], params: Some("argument_list"), params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "function_definition", name: &["identifier", "match", "case", "type"], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];

/// JavaScript's declarations. Nothing in this table is a TypeScript-only
/// construct: `interface`, `enum` and `type` are names in a `.js` file, and a
/// reader that treated them as declarations would invent symbols the CST does
/// not have. They live in [`TS_ONLY_DEFS`], and the assertion below keeps the
/// two tables from ever meeting.
const JS_DEFS: &[Def] = &[
    Def { kind: "class_declaration", name: &["identifier"], params: Some("class_heritage"), params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "class", name: &["identifier"], params: Some("class_heritage"), params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "function_declaration", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "generator_function_declaration", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "method_definition", name: &["property_identifier", "identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "function_expression", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "arrow_function", name: &[], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];

/// The declarations only TypeScript has: an interface, an enum and a type
/// alias. `export type Check = (a: number) => boolean;` declares a named type
/// and the one-pass lexer opens a symbol for it, which is why TypeScript reads
/// this table and JavaScript does not.
const TS_ONLY_DEFS: &[Def] = &[
    Def { kind: "interface_declaration", name: &["type_identifier", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_declaration", name: &["identifier", "type_identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "type_alias_declaration", name: &["type_identifier", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: false },
];

/// TypeScript's declarations: JavaScript's, then the constructs only it has.
const TS_DEFS: &[Def] = &[
    Def { kind: "class_declaration", name: &["identifier"], params: Some("class_heritage"), params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "class", name: &["identifier"], params: Some("class_heritage"), params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "interface_declaration", name: &["type_identifier", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_declaration", name: &["identifier", "type_identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "type_alias_declaration", name: &["type_identifier", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "function_declaration", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "generator_function_declaration", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "method_definition", name: &["property_identifier", "identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "function_expression", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "arrow_function", name: &[], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];

/// Byte equality, so the disjointness check runs in a `const` context where
/// `str` comparison is not available.
const fn same_name(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut at = 0;
    while at < a.len() {
        if a[at] != b[at] {
            return false;
        }
        at += 1;
    }
    true
}

/// Whether any declaration kind appears in both tables.
const fn defs_overlap(a: &[Def], b: &[Def]) -> bool {
    let mut i = 0;
    while i < a.len() {
        let mut j = 0;
        while j < b.len() {
            if same_name(a[i].kind, b[j].kind) {
                return true;
            }
            j += 1;
        }
        i += 1;
    }
    false
}

/// The build-time guarantee: a TypeScript-only declaration kind in JavaScript's
/// table would be read as a JS definition, and the JS reader would name a
/// construct only TypeScript has. Adding one to `JS_DEFS` fails the build here.
const _: () = assert!(
    !defs_overlap(JS_DEFS, TS_ONLY_DEFS),
    "a TypeScript-only kind is in JavaScript's definition table"
);

/// Go has no class declaration: a `struct` or `interface` is a type, and the
/// lexer keeps it as a statement. A `func` with a receiver list is a method, and
/// its owner is the receiver's type.
const GO_DEFS: &[Def] = &[
    Def { kind: "function_declaration", name: &["identifier"], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "method_declaration", name: &["identifier"], params: Some("parameter_list"), params_nth: 1, name_last: false, receiver: true, container: false },
];

/// Java's `class`, `interface`, `enum`, `record` and `@interface` are the
/// containers the lexer opens a class frame for; a method's name is the last
/// identifier in its header, because the return type comes first.
const JAVA_DEFS: &[Def] = &[
    Def { kind: "class_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "interface_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "record_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "annotation_type_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "method_declaration", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: true, receiver: false, container: false },
    Def { kind: "constructor_declaration", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];

const PY_CALLS: &[&str] = &["call"];
const JS_CALLS: &[&str] = &["call_expression", "new_expression"];
const PY_IMPORTS: &[&str] = &["import_statement", "import_from_statement"];
const JS_IMPORTS: &[&str] = &["import_statement", "export_statement"];
/// C#, Kotlin, Scala and Swift: one reader (this walk), one table each. The
/// declarations differ only in the node kinds and where the name sits.
const CS_DEFS: &[Def] = &[
    Def { kind: "class_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "struct_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "interface_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "record_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "method_declaration", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: true, receiver: false, container: false },
    Def { kind: "constructor_declaration", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "delegate_declaration", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "local_function_statement", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const CS_CALLS: &[&str] = &[
    "invocation_expression",
    "object_creation_expression",
    "object_creation",
    "conditional_invocation",
];
const CS_IMPORTS: &[&str] = &["using_directive"];
/// C#'s contextual keywords: `value` is the implicit parameter of a property
/// or event accessor and an ordinary name everywhere else, so treating it as
/// layout dropped `value` from every `set` guard and assignment.
// `value` and `remove` are C# contextual keywords that also name parameters and locals.
const CS_SOFT: &[&str] = &["value", "remove"];

/// Java's contextual keywords: `module`, `open`, `requires`, `exports`,
/// `opens`, `to`, `uses`, `provides`, `with` and `transitive` are keywords only
/// inside a `module-info.java` directive, and `record`, `yield`, `sealed`,
/// `permits` and `when` only where a declaration or a statement claims them.
/// Everywhere else the grammar's `_name` rule accepts them and the lexer reads
/// them as names; without this list a leaf whose kind is its own spelling was
/// layout, so `swap(array, from, to)` lost its third argument and `to = x`
/// lost its target (53 rows in the java dump).
const JAVA_SOFT: &[&str] = &[
    "exports", "module", "open", "opens", "permits", "provides", "record", "requires", "sealed", "to",
    "transitive", "uses", "when", "with", "yield",
];

const KT_DEFS: &[Def] = &[
    Def { kind: "class_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "interface_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_class_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "object_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "function_declaration", name: &["function_head"], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const KT_CALLS: &[&str] = &["call_expression"];
/// Kotlin's soft keywords: every word the grammar's `_name` rule accepts is a
/// name wherever a name can sit (`val value = f()`, `fun open() {}`), and the
/// token the tree keeps for one is the keyword itself rather than an
/// `identifier`. Without the list a property called `value`, `data` or `open`
/// read as a head with no name, so its initialiser's call never reached the
/// record — the largest kotlin cluster the guard still saw.
const KT_SOFT: &[&str] = &[
    "abstract", "actual", "annotation", "by", "companion", "const", "constructor", "context", "crossinline",
    "data", "delegate", "dynamic", "enum", "expect", "external", "field", "final", "get", "infix", "init",
    "inline", "inner", "internal", "lateinit", "noinline", "open", "operator", "out", "override", "private",
    "protected", "public", "reified", "sealed", "set", "suspend", "tailrec", "value", "vararg", "where",
];
const KT_IMPORTS: &[&str] = &["import_header", "package_header"];

const SC_DEFS: &[Def] = &[
    Def { kind: "class_definition", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "trait_definition", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "object_definition", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "def_definition", name: &["identifier"], params: Some("parameter_clause"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const SC_CALLS: &[&str] = &["call", "call_expression", "new_expression"];
const SC_IMPORTS: &[&str] = &["import_declaration"];

/// Erlang functions: every clause of a function shares one `function_definition`
/// and its name is the atom inside the head. The language had no table at all,
/// so the walk returned a record with nothing in it.
const ERL_DEFS: &[Def] = &[Def {
    kind: "function_definition",
    name: &["function_head"],
    params: Some("parameter_group"),
    params_nth: 0,
    name_last: false,
    receiver: false,
    container: false,
}];
/// `f(x)` and `mod:f(x)`: the grammar keeps the remote form as its own node.
const ERL_CALLS: &[&str] = &["call", "remote_call"];

const SW_DEFS: &[Def] = &[
    Def { kind: "class_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "struct_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "protocol_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "extension_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "function_declaration", name: &["identifier", "set"], params: Some("parameter_clause"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const SW_CALLS: &[&str] = &["call_expression"];
const SW_IMPORTS: &[&str] = &["import_declaration"];

const GO_IMPORTS: &[&str] = &["import_declaration"];
const JAVA_IMPORTS: &[&str] = &["import_declaration"];

// -- Rust and JSON (L3). Tables and walk arms live together below so a field
// -- adding another language touches one region, not this one.

/// Rust declarations the lexer files as symbols. `impl` is not here: its name
/// is the type it implements, not a child kind, so [`Walker::rust_impl`] reads
/// it. A trait's methods take the trait as their owner, which is why a trait is
/// a container and a struct (no methods of its own in the grammar) is too.
const RS_DEFS: &[Def] = &[
    Def { kind: "struct_item", name: &["type_identifier", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_item", name: &["type_identifier", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "union_item", name: &["type_identifier", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "trait_item", name: &["type_identifier", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "mod_item", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "function_item", name: &["identifier", "default", "raw", "auto", "safe", "union", "macro_rules"], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];

/// Rust calls. A macro invocation is not a call to the lexer — `assert_eq!(a)`
/// is a merge of names — so it is read by [`Walker::rust_form`], not here.
const RS_CALLS: &[&str] = &["call_expression"];

/// Rust imports: `use` declarations. `mod` is a declaration, not an import.
const RS_IMPORTS: &[&str] = &["use_declaration"];

/// JSON is data: no definitions, no calls, no imports. The tables are empty on
/// purpose and [`lang_for`] does not answer for `JSON`; the record is the
/// module symbol, which is what a data file can honestly say.
const JSON_DEFS: &[Def] = &[];
const JSON_CALLS: &[&str] = &[];
const JSON_IMPORTS: &[&str] = &[];

// -- C and C++ (L4). The name of a C definition is inside its declarator, so
// -- [`Walker::c_function`] reads it; these tables say which nodes are
// -- definitions at all, and which nodes are calls and imports.

/// C definitions the lexer files as symbols: a function with a body, in each
/// of the shapes the grammar needs for a declared or macro-supplied return
/// type. A prototype is not here — it has no body and the lexer opens nothing.
const C_DEFS: &[Def] = &[
    Def { kind: "function_definition", name: &[], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "user_function_definition", name: &[], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "bare_user_function_definition", name: &[], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "implicit_function", name: &[], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
];

/// C calls: one call expression, with a name, a `->`/`.` member or an index as
/// its callee; the walking of the callee is [`Walker::c_callee`].
const C_CALLS: &[&str] = &["call_expression"];

/// C imports: the preprocessor is trivia, so the only import node that could
/// exist never does. The table names the token the grammar calls it, and
/// [`Walker::c_includes`] reads the directive lines from the source.
const C_IMPORTS: &[&str] = &["preprocessor"];

/// C++ definitions: C's, plus the class, namespace, constructor, destructor and
/// out-of-line method forms. `template_declaration` is not here: it wraps the
/// item it declares, and the wrapped node opens the symbol.
const CPP_DEFS: &[Def] = &[
    Def { kind: "function_definition", name: &[], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "plain_user_function", name: &[], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "specified_user_function", name: &[], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "scoped_user_function", name: &[], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "scoped_member_definition", name: &[], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "scoped_destructor_definition", name: &[], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "constructor_definition", name: &["identifier"], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "destructor_definition", name: &["identifier"], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "class_specifier", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "namespace_definition", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
];

/// C++ calls. `new` is a call to the type it constructs, which is what the
/// lexer reads after the `new` keyword.
const CPP_CALLS: &[&str] = &["call_expression", "new_expression"];

/// C++ imports, as C's: `#include` is a directive line, not a node.
const CPP_IMPORTS: &[&str] = &["preprocessor"];

/// The visible declaration wrappers, each owning a declarator the plain
/// `declaration_statement` does not: a user type (`bare_user_declaration`), a
/// typedef name, a specifier-qualified one, or a C++ constructor declaration.
const C_DECLARATIONS: &[&str] = &[
    "bare_user_declaration",
    "specified_user_declaration",
    "typedef_user_declaration",
    "constructor_declaration",
];


/// Everything the walk needs to know about one language.
/// Python's soft keywords: `match x:` is a statement, `match.group` is an
/// attribute of a name, and only the position tells them apart. This grammar
/// lexes both as the keyword kind, so the mapping has to read the leaf.
const PY_SOFT: &[&str] = &["match", "case", "type", "_"];

/// JavaScript's contextual keywords. Each is an ordinary identifier in the
/// grammar unless a statement claims it, so a leaf whose kind is `from` is a
/// name (`g(from)`, `let from = 1`) and not punctuation. Reading them as layout
/// dropped every argument or binding spelled that way.
const JS_SOFT: &[&str] = &["from", "of", "get", "set", "module", "global", "type", "unique", "accessor", "namespace", "readonly"];

/// Rust's contextual keywords: each is a name everywhere but where it starts
/// its own item, and the grammar's `_name` rule accepts every one of them. The
/// lexer reads them as names, so without this the CST dropped `raw` in
/// `match raw[0]` as punctuation and lost the value it names.
const RS_SOFT: &[&str] = &["union", "default", "raw", "auto", "safe", "macro_rules"];


// -- Wave 1 language tables (Dart, Haskell, Solidity, Ruby, PHP, Lua, Crystal, F#, Tcl, R, PowerShell) --

const DART_DEFS: &[Def] = &[
    Def { kind: "class_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "mixin_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "extension_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "function_declaration", name: &["identifier"], params: Some("parameter_list"), params_nth: 0, name_last: true, receiver: false, container: false },
];
const DART_CALLS: &[&str] = &["call", "const_object_expression"];
const DART_IMPORTS: &[&str] = &["import_directive"];

const HS_DEFS: &[Def] = &[
    Def { kind: "data_declaration", name: &["constructor"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "newtype_declaration", name: &["constructor"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "type_declaration", name: &["constructor"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "class_declaration", name: &["constructor"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "function_binding", name: &["variable"], params: None, params_nth: 0, name_last: false, receiver: false, container: false },
];
const HS_IMPORTS: &[&str] = &["import_declaration"];

const SOL_DEFS: &[Def] = &[
    Def { kind: "contract_definition", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "struct_definition", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_definition", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "function_definition", name: &["identifier"], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "free_function", name: &["identifier"], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "constructor_definition", name: &["constructor"], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "modifier_definition", name: &["identifier"], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const SOL_CALLS: &[&str] = &["call", "payable_call"];
const SOL_IMPORTS: &[&str] = &["import_directive"];

const RB_DEFS: &[Def] = &[
    Def { kind: "class_definition", name: &["constant", "scope"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "module_definition", name: &["constant", "scope"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "method_definition", name: &["method_name", "identifier"], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const RB_CALLS: &[&str] = &["call", "command_call", "bare_call", "super_call", "defined_expression"];
const RB_IMPORTS: &[&str] = &["require_statement"];

const PHP_DEFS: &[Def] = &[
    Def { kind: "class_declaration", name: &["name"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "interface_declaration", name: &["name"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "trait_declaration", name: &["name"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_declaration", name: &["name"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "method_declaration", name: &["name"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "function_declaration", name: &["name"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const PHP_CALLS: &[&str] = &["function_call_expression", "new_expression"];
const PHP_IMPORTS: &[&str] = &["use_declaration", "trait_use_declaration", "include_expression"];

const LUA_DEFS: &[Def] = &[
    Def { kind: "function_declaration", name: &["function_name", "identifier"], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "local_function", name: &["identifier"], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "function_expression", name: &[], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const LUA_CALLS: &[&str] = &["call", "method_call"];

const CR_DEFS: &[Def] = &[
    Def { kind: "class_definition", name: &["constant", "scope"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "module_definition", name: &["constant", "scope"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "struct_definition", name: &["constant", "scope"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_definition", name: &["constant", "scope"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "lib_definition", name: &["constant", "scope"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "method_definition", name: &["method_name", "identifier"], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "fun_declaration", name: &["identifier"], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const CR_CALLS: &[&str] = &["call", "command_call", "super_call"];
const CR_IMPORTS: &[&str] = &["require_statement"];

const FS_DEFS: &[Def] = &[
    Def { kind: "module_declaration", name: &["qualified_name", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "namespace_declaration", name: &["qualified_name", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "type_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "let_binding", name: &["binding_pattern", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: false },
];
const FS_CALLS: &[&str] = &["application"];
const FS_IMPORTS: &[&str] = &["open_declaration"];

const TCL_DEFS: &[Def] = &[
    Def { kind: "proc_command", name: &["word"], params: None, params_nth: 0, name_last: false, receiver: false, container: false },
];
const TCL_CALLS: &[&str] = &["call_command"];

const R_CALLS: &[&str] = &["call"];

const PS1_DEFS: &[Def] = &[
    Def { kind: "class_definition", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_definition", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "function_definition", name: &["identifier"], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const PS1_CALLS: &[&str] = &["command_expression"];


const AWK_DEFS: &[Def] = &[
    Def { kind: "function_definition", name: &["identifier"], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const AWK_CALLS: &[&str] = &["call"];

const PAS_DEFS: &[Def] = &[
    Def { kind: "routine_declaration", name: &["identifier"], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const PAS_CALLS: &[&str] = &["call"];
const PAS_IMPORTS: &[&str] = &["uses_clause"];

const SH_DEFS: &[Def] = &[
    Def { kind: "function_definition", name: &["function_name", "word", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "paren_function", name: &["function_name", "word", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: false },
];
const SH_CALLS: &[&str] = &["simple_command"];

const ZIG_DEFS: &[Def] = &[
    Def { kind: "fn_declaration", name: &["identifier"], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const ZIG_CALLS: &[&str] = &["call", "builtin_call"];

const PS1_IMPORTS: &[&str] = &["using_statement"];

const NIM_DEFS: &[Def] = &[
    Def { kind: "routine_definition", name: &["identifier", "operator_name"], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "type_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
];
const NIM_CALLS: &[&str] = &["call", "command_call"];

const GROOVY_DEFS: &[Def] = &[
    Def { kind: "class_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "interface_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "trait_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "definition", name: &["identifier"], params: Some("parameter_list"), params_nth: 0, name_last: true, receiver: false, container: false },
    Def { kind: "field_or_method", name: &["identifier"], params: Some("parameter_list"), params_nth: 0, name_last: true, receiver: false, container: false },
];
const GROOVY_CALLS: &[&str] = &["call", "constructor_call"];
const GROOVY_IMPORTS: &[&str] = &[];

fn kinds(lang: Lang) -> Kinds {
    match lang {
        Lang::Python => {
            Kinds { definitions: PY_DEFS, calls: PY_CALLS, imports: PY_IMPORTS, soft: PY_SOFT, family: Family::Plain }
        }
        // Objective-C keeps the C tables: `@interface` / `@implementation`
        // are the C reader's containers, and `[receiver selector]` is a
        // postfix the shared walk reads like a call.
        Lang::C | Lang::Cpp | Lang::ObjC => Kinds {
            definitions: if lang == Lang::Cpp { CPP_DEFS } else { C_DEFS },
            calls: if lang == Lang::Cpp { CPP_CALLS } else { C_CALLS },
            imports: if lang == Lang::Cpp { CPP_IMPORTS } else { C_IMPORTS },
            soft: &[],
            family: Family::Plain,
        },
        Lang::Rust => {
            Kinds { definitions: RS_DEFS, calls: RS_CALLS, imports: RS_IMPORTS, soft: RS_SOFT, family: Family::Rust }
        }
        Lang::Json => {
            Kinds { definitions: JSON_DEFS, calls: JSON_CALLS, imports: JSON_IMPORTS, soft: &[], family: Family::Plain }
        }
        Lang::JavaScript => {
            Kinds { definitions: JS_DEFS, calls: JS_CALLS, imports: JS_IMPORTS, soft: JS_SOFT, family: Family::Js }
        }
        Lang::TypeScript | Lang::Tsx => {
            Kinds { definitions: TS_DEFS, calls: JS_CALLS, imports: JS_IMPORTS, soft: JS_SOFT, family: Family::Js }
        }
        // Go and Java write their statements and their calls themselves: the
        // lexer's statement model is mirrored by `decl_value` rather than by
        // `form`, so the generic arms below are not reached.
        Lang::Go => Kinds { definitions: GO_DEFS, calls: &[], imports: GO_IMPORTS, soft: &[], family: Family::Go },
        Lang::Java => Kinds { definitions: JAVA_DEFS, calls: &[], imports: JAVA_IMPORTS, soft: JAVA_SOFT, family: Family::Java },
        Lang::CSharp => Kinds { definitions: CS_DEFS, calls: CS_CALLS, imports: CS_IMPORTS, soft: CS_SOFT, family: Family::Plain },
        Lang::Kotlin => {
            Kinds { definitions: KT_DEFS, calls: KT_CALLS, imports: KT_IMPORTS, soft: KT_SOFT, family: Family::Plain }
        }
        Lang::Scala => Kinds { definitions: SC_DEFS, calls: SC_CALLS, imports: SC_IMPORTS, soft: &[], family: Family::Plain },
        Lang::Swift => Kinds { definitions: SW_DEFS, calls: SW_CALLS, imports: SW_IMPORTS, soft: &[], family: Family::Plain },
        Lang::Erlang => {
            Kinds { definitions: ERL_DEFS, calls: ERL_CALLS, imports: &[], soft: &[], family: Family::Plain }
        }
        Lang::Dart => {
            Kinds { definitions: DART_DEFS, calls: DART_CALLS, imports: DART_IMPORTS, soft: &[], family: Family::Plain }
        }
        Lang::Haskell => {
            Kinds { definitions: HS_DEFS, calls: &[], imports: HS_IMPORTS, soft: &[], family: Family::Plain }
        }
        Lang::Solidity => {
            Kinds { definitions: SOL_DEFS, calls: SOL_CALLS, imports: SOL_IMPORTS, soft: &[], family: Family::Plain }
        }
        Lang::Ruby => {
            Kinds { definitions: RB_DEFS, calls: RB_CALLS, imports: RB_IMPORTS, soft: &[], family: Family::Plain }
        }
        Lang::Php => {
            Kinds { definitions: PHP_DEFS, calls: PHP_CALLS, imports: PHP_IMPORTS, soft: &[], family: Family::Plain }
        }
        Lang::Lua => {
            Kinds { definitions: LUA_DEFS, calls: LUA_CALLS, imports: &[], soft: &[], family: Family::Plain }
        }
        Lang::Crystal => {
            Kinds { definitions: CR_DEFS, calls: CR_CALLS, imports: CR_IMPORTS, soft: &[], family: Family::Plain }
        }
        Lang::FSharp => {
            Kinds { definitions: FS_DEFS, calls: FS_CALLS, imports: FS_IMPORTS, soft: &[], family: Family::Plain }
        }
        Lang::Tcl => {
            Kinds { definitions: TCL_DEFS, calls: TCL_CALLS, imports: &[], soft: &[], family: Family::Plain }
        }
        Lang::R => {
            Kinds { definitions: &[], calls: R_CALLS, imports: &[], soft: &[], family: Family::Plain }
        }
        Lang::PowerShell => {
            Kinds { definitions: PS1_DEFS, calls: PS1_CALLS, imports: PS1_IMPORTS, soft: &[], family: Family::Plain }
        }
        Lang::Zig => {
            Kinds { definitions: ZIG_DEFS, calls: ZIG_CALLS, imports: &[], soft: &[], family: Family::Plain }
        }
        Lang::Awk => {
            Kinds { definitions: AWK_DEFS, calls: AWK_CALLS, imports: &[], soft: &[], family: Family::Plain }
        }
        Lang::Pascal => {
            Kinds { definitions: PAS_DEFS, calls: PAS_CALLS, imports: PAS_IMPORTS, soft: &[], family: Family::Plain }
        }
        Lang::Shell => {
            Kinds { definitions: SH_DEFS, calls: SH_CALLS, imports: &[], soft: &[], family: Family::Plain }
        }
        Lang::Nim => {
            Kinds { definitions: NIM_DEFS, calls: NIM_CALLS, imports: &[], soft: &[], family: Family::Plain }
        }
        Lang::Groovy => {
            Kinds { definitions: GROOVY_DEFS, calls: GROOVY_CALLS, imports: GROOVY_IMPORTS, soft: &[], family: Family::Plain }
        }
        // The other grammars have no definition table here yet; the walk still
        // returns a record with the module symbol, which is what the caller
        // falls back on. Named so a caller can tell "no facts" from "empty file".
        _ => Kinds { definitions: &[], calls: &[], imports: &[], soft: &[], family: Family::Plain },
    }
}

/// How many declaration kinds this language's table names. Zero means the walk
/// has nothing to read for it, which is a missing table, not an empty file.
pub fn definition_kinds(lang: Lang) -> usize {
    kinds(lang).definitions.len()
}

/// The lexer's tag for a language, so both paths write the same `tag`.
fn tag(lang: Lang) -> &'static str {
    match lang {
        Lang::Json => "JSON",
        Lang::Python => "PY",
        Lang::Rust => "RS",
        Lang::JavaScript => "JS",
        Lang::TypeScript | Lang::Tsx => "TS",
        Lang::Go => "GO",
        Lang::Java => "JAVA",
        Lang::C => "C",
        Lang::Cpp => "CPP",
        Lang::CSharp => "CS",
        Lang::Kotlin => "KT",
        Lang::Scala => "SCALA",
        Lang::Swift => "SWIFT",
        Lang::ObjC => "OBJC",
        Lang::Php => "PHP",
        Lang::Ruby => "RB",
        Lang::Dart => "DART",
        Lang::Elixir => "EX",
        Lang::Haskell => "HS",
        Lang::Lua => "LUA",
        Lang::R => "R",
        Lang::Solidity => "SOL",
        Lang::Sql => "SQL",
        Lang::Shell => "SH",
        Lang::Groovy => "GROOVY",
        Lang::Julia => "JL",
        Lang::VisualBasic => "VB",
        Lang::Pascal => "PAS",
        Lang::Cobol => "COBOL",
        Lang::Nim => "NIM",
        Lang::Crystal => "CR",
        Lang::Zig => "ZIG",
        Lang::D => "D",
        Lang::FSharp => "FS",
        Lang::Ocaml => "ML",
        Lang::Elm => "ELM",
        Lang::Matlab => "M",
        Lang::Erlang => "ERL",
        Lang::Clojure => "CLJ",
        Lang::Tcl => "TCL",
        Lang::Awk => "AWK",
        Lang::Perl => "PL",
        Lang::Fortran => "F90",
        Lang::PowerShell => "PS1",
    }
}

// ---------------------------------------------------------------------------
// The walk.
// ---------------------------------------------------------------------------

/// What the walk is inside: which symbol owns new calls and events, which class
/// a member belongs to, and how this node's `end_line` is set on the way out.
struct Frame {
    node: u32,
    /// The symbol that receives calls and events written while inside this
    /// node. A class body writes onto the enclosing symbol, exactly as the
    /// lexer does: `class A: X = f()` puts that call on the module.
    symbol: usize,
    class: String,
    /// Go's method receiver's own name (`c` in `func (c *Cache) Get`). The
    /// lexer spells `c.mu` as `self.mu`, so the path builder does too.
    receiver: String,
    container: bool,
    defines: Option<usize>,
}

struct Walker<'a> {
    src: &'a [u8],
    /// Unpatched source bytes when `cs_strip_primary_constructors` rewrote C# 12
    /// syntax (such as `[x]` -> `(x)`) before parsing, so `parenthesized_expression`
    /// can tell a single-element collection expression from real parentheses.
    orig_src: &'a [u8],
    nodes: &'a [Node],
    language: &'static Language,
    kinds: Kinds,
    line_starts: Vec<u32>,
    facts: Facts,
    stack: Vec<Frame>,
    /// `const f = () => {}` / `f = lambda ...`: the definition node and the name
    /// the binding gives it. A function expression has no name of its own; the
    /// binding is the name, exactly as the lexer reads it.
    bound: Vec<(u32, String)>,
    /// Nodes whose statement an enclosing arm already wrote as one event: the
    /// members of an enum are a single assignment to the lexer, so visiting
    /// each one on its own must not write a second, unpaired event.
    consumed: Vec<u32>,
    /// `hidden void f(void);`: the macro that opened the declaration, read by
    /// the item's own node and spent on the declaration inside it.
    macro_prefix: Option<String>,
    /// The line a declaration's statement starts on, by declaration node: the
    /// lexer opens a statement at the first token after the previous `;`, so
    /// `TEST_END` and `seq_define(…)` — macro invocations with no `;` of their
    /// own — put the declaration that follows them on the macro's line while
    /// the declaration's own line is the later one (c corpus, 6 files).
    statement_lines: std::collections::HashMap<u32, u32>,
    /// The form text a call node already produced, by node id. A block value is
    /// read twice — once for the value the assignment carries, once when the
    /// walk reaches the statements inside it — and the second read pushed a
    /// second `Call` for the same source call. Returning the first text keeps
    /// one call and one index, which is what the text in the event refers to
    /// (P5-11).
    call_texts: std::collections::HashMap<u32, String>,
    in_kotlin_expr_body: bool,
    /// Depth inside `(...)` or `[...]` while forming expressions; the lexer
    /// does not split a multiline binary expression at a newline inside
    /// delimiters.
    delim_depth: usize,
    /// `name := func(...) { ... }` / `var name = func(...) { ... }`: maps the
    /// `func_literal` node to the function symbol created for its binding so
    /// the walk enters that symbol's frame for the literal's body.
    go_literal_frames: std::collections::HashMap<u32, usize>,
}

/// The deep language for a lexer tag (`langs::Language::tag`), or `None` when
/// this path has no grammar for it. The pipeline uses this to pick the deep
/// path for exactly the files whose facts it can produce.
pub fn lang_for(tag: &str) -> Option<Lang> {
    // `auto` picks only languages whose record reliably beats the lexer. Go and
    // Java have tables (see `lang_for_forced`) but their records still lose the
    // lexer's artefacts often enough that the guard keeps the lexer on most
    // files, so choosing them automatically would pay for a second parse and
    // then throw it away.
    // `RG_AUDIT=1` asks for the guard's verdict on every language this build can
    // parse, including the ones `auto` will not pay a second parse for. The
    // audit that decides whether a language is ready to enter `auto` needs that
    // number, and `--deep` cannot give it: it forces the CST on every file
    // instead of comparing the two readers. Measurement only, off by default.
    if std::env::var_os("RG_AUDIT").is_some() {
        return lang_for_forced(tag);
    }
    match tag {
        "PY" | "JS" | "TS" | "RS" => lang_for_forced(tag),
        _ => None,
    }
}

/// The languages this build can parse at all, whether or not `auto` would pick
/// them. `--deep` uses this, so a caller can still ask for the CST on a language
/// whose record is better for calls than the lexer's even where the guard would
/// keep the lexer.
pub fn lang_for_forced(tag: &str) -> Option<Lang> {
    match tag {
        "PY" => Some(Lang::Python),
        "JS" => Some(Lang::JavaScript),
        "TS" => Some(Lang::TypeScript),
        "RS" => Some(Lang::Rust),
        "GO" => Some(Lang::Go),
        "JAVA" => Some(Lang::Java),
        "C" => Some(Lang::C),
        "CPP" => Some(Lang::Cpp),
        "CS" => Some(Lang::CSharp),
        "KT" => Some(Lang::Kotlin),
        "SCALA" => Some(Lang::Scala),
        "SWIFT" => Some(Lang::Swift),
        "OBJC" => Some(Lang::ObjC),
        "PHP" => Some(Lang::Php),
        "RB" => Some(Lang::Ruby),
        "DART" => Some(Lang::Dart),
        "EX" => Some(Lang::Elixir),
        "HS" => Some(Lang::Haskell),
        "LUA" => Some(Lang::Lua),
        "R" => Some(Lang::R),
        "SOL" => Some(Lang::Solidity),
        "SQL" => Some(Lang::Sql),
        "SH" => Some(Lang::Shell),
        "GROOVY" => Some(Lang::Groovy),
        "JL" => Some(Lang::Julia),
        "VB" => Some(Lang::VisualBasic),
        "PAS" => Some(Lang::Pascal),
        "COBOL" => Some(Lang::Cobol),
        "NIM" => Some(Lang::Nim),
        "CR" => Some(Lang::Crystal),
        "ZIG" => Some(Lang::Zig),
        "D" => Some(Lang::D),
        "FS" => Some(Lang::FSharp),
        "ML" => Some(Lang::Ocaml),
        "ELM" => Some(Lang::Elm),
        "M" => Some(Lang::Matlab),
        "ERL" => Some(Lang::Erlang),
        "CLJ" => Some(Lang::Clojure),
        "TCL" => Some(Lang::Tcl),
        "AWK" => Some(Lang::Awk),
        "PL" => Some(Lang::Perl),
        "PS1" => Some(Lang::PowerShell),
        "F90" => Some(Lang::Fortran),
        // JSON stays out: a data file has no symbols, calls or imports to
        // read, so the CST would add the module symbol the lexer already
        // writes and nothing else. See `tests/facts_rust_json.rs`.
        _ => None,
    }
}

/// Read a file's facts off its CST.
pub fn extract(root: &[u8], tree: &Cst, lang: Lang) -> Facts {
    // C# 12 primary constructors on `class` and `struct` (`class C(int x) { ... }`)
    // are not in the C# grammar yet, so tree-sitter terminates the class at
    // its name and dumps `(x) { ... }` into an `ERROR` region. Blanking the
    // parameter list (and any `: Base(x)` argument list) with spaces preserves
    // every line number and lets the CST read the class body normally.
    if lang == Lang::CSharp && tree.errors > 0 {
        if let Some(patched) = cs_strip_primary_constructors(root) {
            if let Some(re_tree) = crate::deep_integration::parse_file(&patched, lang) {
                if re_tree.errors < tree.errors || re_tree.nodes.len() > tree.nodes.len() {
                    let mut walker = Walker::new(&patched, &re_tree, lang);
                    walker.orig_src = root;
                    walker.run();
                    return walker.facts;
                }
            }
        }
    }
    let mut walker = Walker::new(root, tree, lang);
    walker.run();
    if lang == Lang::Ruby {
        walker.augment_ruby();
    }
    if lang == Lang::Tcl {
        walker.augment_tcl();
    }
    if lang == Lang::Awk {
        walker.augment_awk();
    }
    if lang == Lang::PowerShell {
        walker.augment_powershell();
    }
    if lang == Lang::Nim {
        walker.augment_nim();
    }
    if lang == Lang::Solidity {
        walker.augment_solidity();
    }
    if lang == Lang::Php {
        walker.augment_php();
    }
    if lang == Lang::C {
        walker.augment_c();
    }
    if lang == Lang::Swift {
        walker.augment_swift();
    }
    if lang == Lang::Rust {
        walker.augment_rust();
    }
    if lang == Lang::Scala {
        walker.augment_scala();
    }
    if lang == Lang::Lua {
        walker.augment_lua();
    }
    if lang == Lang::Perl {
        walker.augment_perl();
    }
    if lang == Lang::Dart {
        walker.augment_dart();
    }
    if lang == Lang::Shell {
        walker.augment_shell();
    }
    if lang == Lang::CSharp {
        walker.augment_csharp();
    }
    if lang == Lang::FSharp {
        walker.augment_fsharp();
    }
    if lang == Lang::Zig {
        walker.augment_zig();
    }
    if matches!(lang, Lang::JavaScript | Lang::TypeScript | Lang::Tsx) {
        walker.augment_js();
    }
    if lang == Lang::Python {
        walker.augment_python();
    }
    if lang == Lang::Pascal {
        walker.augment_pascal();
    }
    if lang == Lang::Clojure {
        walker.augment_clojure();
    }
    if lang == Lang::VisualBasic {
        walker.augment_vb();
    }
    if lang == Lang::Cpp {
        walker.augment_cpp();
    }
    if lang == Lang::Cobol {
        walker.augment_cobol();
    }
    if lang == Lang::Fortran {
        walker.augment_fortran();
    }
    if lang == Lang::Sql {
        walker.augment_sql();
    }
    if lang == Lang::Elixir {
        walker.augment_elixir();
    }
    if lang == Lang::Ocaml {
        walker.augment_ocaml();
    }
    if lang == Lang::D {
        walker.augment_d();
    }
    if lang == Lang::Julia {
        walker.augment_julia();
    }
    if lang == Lang::Elm {
        walker.augment_elm();
    }
    if lang == Lang::Matlab {
        walker.augment_matlab();
    }
    if lang == Lang::Erlang {
        walker.augment_erlang();
    }
    if lang == Lang::Haskell {
        walker.augment_haskell();
    }
    if lang == Lang::ObjC {
        walker.augment_objc_full();
    }
    if lang == Lang::Kotlin {
        walker.augment_kotlin();
    }
    if lang == Lang::Java {
        walker.augment_java();
    }
    walker.facts





}

fn cs_skip_ws_and_comments(src: &[u8], mut i: usize) -> usize {
    while i < src.len() {
        if src[i].is_ascii_whitespace() {
            i += 1;
        } else if src[i] == b'#' {
            i += 1;
            while i < src.len() && src[i] != b'\n' {
                i += 1;
            }
        } else if src.get(i..i + 2) == Some(b"//") {
            i += 2;
            while i < src.len() && src[i] != b'\n' {
                i += 1;
            }
        } else if src.get(i..i + 2) == Some(b"/*") {
            i += 2;
            while i + 1 < src.len() && src.get(i..i + 2) != Some(b"*/") {
                i += 1;
            }
            i = (i + 2).min(src.len());
        } else {
            break;
        }
    }
    i
}

fn cs_skip_string_or_char(src: &[u8], i: usize) -> Option<usize> {
    if src.get(i..i + 2) == Some(b"@\"") {
        let mut j = i + 2;
        while j < src.len() {
            if src[j] == b'"' {
                if src.get(j + 1) == Some(&b'"') {
                    j += 2;
                } else {
                    return Some(j + 1);
                }
            } else {
                j += 1;
            }
        }
        return Some(src.len());
    }
    let quote = src[i];
    if quote == b'"' || quote == b'\'' {
        let mut j = i + 1;
        while j < src.len() {
            if src[j] == b'\\' {
                j += 2;
            } else if src[j] == quote {
                return Some(j + 1);
            } else {
                j += 1;
            }
        }
        return Some(src.len());
    }
    None
}

fn cs_skip_balanced(src: &[u8], i: usize, open: u8, close: u8) -> Option<usize> {
    if src.get(i) != Some(&open) {
        return None;
    }
    let mut depth = 1usize;
    let mut j = i + 1;
    while j < src.len() {
        if let Some(next) = cs_skip_string_or_char(src, j) {
            j = next;
            continue;
        }
        if src.get(j..j + 2) == Some(b"//") || src.get(j..j + 2) == Some(b"/*") {
            j = cs_skip_ws_and_comments(src, j);
            continue;
        }
        if src[j] == open {
            depth += 1;
        } else if src[j] == close {
            depth -= 1;
            if depth == 0 {
                return Some(j + 1);
            }
        }
        j += 1;
    }
    None
}

fn cs_is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'@'
}

fn cs_is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn cs_strip_primary_constructors(src: &[u8]) -> Option<Vec<u8>> {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut i = 0usize;
    let mut prev_word: &[u8] = b"";

    while i < src.len() {
        if let Some(next) = cs_skip_string_or_char(src, i) {
            i = next;
            prev_word = b"";
            continue;
        }
        if src[i] == b'#' || src.get(i..i + 2) == Some(b"//") || src.get(i..i + 2) == Some(b"/*") {
            i = cs_skip_ws_and_comments(src, i);
            continue;
        }
        // `[...] = expr`: a dictionary indexer initializer in an object
        // initializer is not in the C# grammar yet, so blanking `[` and `]`
        // turns `["k"] = v` into `"k" = v` without shifting any line.
        if src[i] == b'[' {
            if let Some(after_br) = cs_skip_balanced(src, i, b'[', b']') {
                let after_ws = cs_skip_ws_and_comments(src, after_br);
                if src.get(after_ws) == Some(&b'=') && src.get(after_ws + 1) != Some(&b'=') && src.get(after_ws + 1) != Some(&b'>') {
                    spans.push((i, i + 1));
                    spans.push((after_br - 1, after_br));
                    i = after_br;
                    prev_word = b"";
                    continue;
                }
            }
        }
        // `= [];`, `??= [];`, `=> [];`, `return [];`: an empty C# 12 collection
        // expression has no rule in the grammar yet, and replacing `[]` with
        // `""` (two bytes for two bytes) lets the surrounding class or method
        // parse cleanly with the same constant `0` value.
        if src.get(i..i + 2) == Some(b"[]") {
            let mut p = i;
            while p > 0 && src[p - 1].is_ascii_whitespace() {
                p -= 1;
            }
            let preceded_by_val = p > 0
                && (src[p - 1] == b'='
                    || src[p - 1] == b'('
                    || src[p - 1] == b','
                    || src[p - 1] == b':'
                    || src[p - 1] == b'?'
                    || src[p - 1] == b'['
                    || (p >= 2 && &src[p - 2..p] == b"=>")
                    || (p >= 6 && &src[p - 6..p] == b"return" && (p == 6 || !cs_is_ident_cont(src[p - 7]))));
            if preceded_by_val {
                let after_ws = cs_skip_ws_and_comments(src, i + 2);
                if matches!(src.get(after_ws), Some(b';' | b',' | b')' | b'}' | b']' | b'.')) {
                    spans.push((i, i + 2));
                    i += 2;
                    prev_word = b"";
                    continue;
                }
            }
        }
        // `= [a, b];`, `=> [a, b];`, `return [a, b];`, `Foo(a, [b, c])`: a
        // non-empty C# 12 collection expression `[...]` in value position has
        // no grammar rule yet, and replacing `[` and `]` with `(` and `)`
        // parses as a parenthesized tuple with the exact `t:[...]` flow form.
        // Named arguments (`messages: [a]`), nested collections (`[[a], b]`),
        // and spread elements (`[..a, b]`) are included; top-level `..`
        // spreads are blanked to spaces so `(  a, b)` parses without ERROR.
        if src[i] == b'[' {
            let mut p = i;
            while p > 0 && src[p - 1].is_ascii_whitespace() {
                p -= 1;
            }
            let preceded_by_val = p > 0
                && ((src[p - 1] == b'=' && (p < 2 || !matches!(src[p - 2], b'=' | b'!' | b'<' | b'>')))
                    || src[p - 1] == b'('
                    || src[p - 1] == b','
                    || src[p - 1] == b':'
                    || src[p - 1] == b'['
                    || (src[p - 1] == b'?' && (p < 2 || src[p - 2] != b'?'))
                    || (p >= 2 && &src[p - 2..p] == b"??")
                    || (p >= 2 && &src[p - 2..p] == b"=>")
                    || (p >= 6 && &src[p - 6..p] == b"return" && (p == 6 || !cs_is_ident_cont(src[p - 7]))));
            if preceded_by_val {
                if let Some(after_br) = cs_skip_balanced(src, i, b'[', b']') {
                    let after_ws = cs_skip_ws_and_comments(src, after_br);
                    if matches!(src.get(after_ws), Some(b';' | b',' | b')' | b'}' | b']' | b'.')) {
                        spans.push((i, i + 1));
                        spans.push((after_br - 1, after_br));
                        // Blank top-level `..` spread operators inside `[...]`.
                        let mut k = cs_skip_ws_and_comments(src, i + 1);
                        let end_inner = after_br - 1;
                        if k + 1 < end_inner && &src[k..k + 2] == b".." {
                            spans.push((k, k + 2));
                            k += 2;
                        }
                        while k < end_inner {
                            if let Some(next) = cs_skip_string_or_char(src, k) {
                                k = next;
                                continue;
                            }
                            if src.get(k..k + 2) == Some(b"//") || src.get(k..k + 2) == Some(b"/*") {
                                k = cs_skip_ws_and_comments(src, k);
                                continue;
                            }
                            if let Some(next) = match src[k] {
                                b'(' => cs_skip_balanced(src, k, b'(', b')'),
                                b'[' => cs_skip_balanced(src, k, b'[', b']'),
                                b'{' => cs_skip_balanced(src, k, b'{', b'}'),
                                _ => None,
                            } {
                                k = next;
                                continue;
                            }
                            if src[k] == b',' {
                                k = cs_skip_ws_and_comments(src, k + 1);
                                if k + 1 < end_inner && &src[k..k + 2] == b".." {
                                    spans.push((k, k + 2));
                                    k += 2;
                                }
                                continue;
                            }
                            k += 1;
                        }
                        i += 1;
                        prev_word = b"";
                        continue;
                    }
                }
            }
        }
        if !cs_is_ident_start(src[i]) {
            if !src[i].is_ascii_whitespace() {
                prev_word = b"";
            }
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < src.len() && cs_is_ident_cont(src[i]) {
            i += 1;
        }
        let word = &src[start..i];
        if (word == b"class" || word == b"struct") && prev_word != b"record" {
            let mut j = cs_skip_ws_and_comments(src, i);
            if j < src.len() && cs_is_ident_start(src[j]) {
                j += 1;
                while j < src.len() && cs_is_ident_cont(src[j]) {
                    j += 1;
                }
                j = cs_skip_ws_and_comments(src, j);
                if src.get(j) == Some(&b'<') {
                    if let Some(after_gen) = cs_skip_balanced(src, j, b'<', b'>') {
                        j = cs_skip_ws_and_comments(src, after_gen);
                    }
                }
                if src.get(j) == Some(&b'(') {
                    if let Some(after_params) = cs_skip_balanced(src, j, b'(', b')') {
                        let mut local_spans = vec![(j, after_params)];
                        let mut k = cs_skip_ws_and_comments(src, after_params);
                        // Optional base list `: Base(args), IFoo` before `where` or `{`.
                        if src.get(k) == Some(&b':') {
                            k += 1;
                            while k < src.len() {
                                k = cs_skip_ws_and_comments(src, k);
                                if k >= src.len() || src[k] == b'{' || src[k] == b';' {
                                    break;
                                }
                                if src.get(k) == Some(&b'<') {
                                    if let Some(after_gen) = cs_skip_balanced(src, k, b'<', b'>') {
                                        k = after_gen;
                                        continue;
                                    }
                                }
                                if src.get(k) == Some(&b'(') {
                                    if let Some(after_args) = cs_skip_balanced(src, k, b'(', b')') {
                                        local_spans.push((k, after_args));
                                        k = after_args;
                                        continue;
                                    }
                                }
                                if cs_is_ident_start(src[k]) {
                                    let w_start = k;
                                    k += 1;
                                    while k < src.len() && cs_is_ident_cont(src[k]) {
                                        k += 1;
                                    }
                                    if &src[w_start..k] == b"where" {
                                        break;
                                    }
                                    continue;
                                }
                                k += 1;
                            }
                        }
                        // Advance past optional `where` constraints to confirm `{` body.
                        while k < src.len() {
                            if let Some(next) = cs_skip_string_or_char(src, k) {
                                k = next;
                                continue;
                            }
                            if src.get(k..k + 2) == Some(b"//") || src.get(k..k + 2) == Some(b"/*") {
                                k = cs_skip_ws_and_comments(src, k);
                                continue;
                            }
                            if src[k] == b'{' {
                                spans.extend(local_spans);
                                break;
                            }
                            if src[k] == b';' || src[k] == b'}' {
                                break;
                            }
                            k += 1;
                        }
                    }
                }
            }
        }
        prev_word = word;
    }

    if spans.is_empty() {
        return None;
    }
    let mut patched = src.to_vec();
    for (start, end) in spans {
        if end == start + 2 && &src[start..end] == b"[]" {
            patched[start] = b'"';
            patched[start + 1] = b'"';
            continue;
        }
        if end == start + 1 && src[start] == b'[' {
            let after_ws = cs_skip_ws_and_comments(src, cs_skip_balanced(src, start, b'[', b']').unwrap_or(end));
            if src.get(after_ws) != Some(&b'=') {
                patched[start] = b'(';
                continue;
            }
        }
        if end == start + 1 && src[start] == b']' {
            let after_ws = cs_skip_ws_and_comments(src, end);
            if src.get(after_ws) != Some(&b'=') {
                patched[start] = b')';
                continue;
            }
        }
        for byte in &mut patched[start..end] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    }
    Some(patched)
}

impl<'a> Walker<'a> {
    pub(crate) fn new(src: &'a [u8], tree: &'a Cst, lang: Lang) -> Self {
        let mut facts = Facts { tag: tag(lang).to_string(), ..Facts::default() };
        facts.symbols.push(Symbol { key: "<module>".into(), kind: KIND_MODULE, line: 1, ..Symbol::default() });
        Walker {
            src,
            orig_src: src,
            nodes: &tree.nodes,
            language: lang.language(),
            kinds: kinds(lang),
            line_starts: line_starts(src),
            facts,
            stack: vec![Frame { node: tree.root, symbol: 0, class: String::new(), receiver: String::new(), container: false, defines: None }],
            bound: Vec::new(),
            consumed: Vec::new(),
            macro_prefix: None,
            statement_lines: std::collections::HashMap::new(),
            call_texts: std::collections::HashMap::new(),
            in_kotlin_expr_body: false,
            delim_depth: 0,
            go_literal_frames: std::collections::HashMap::new(),
        }
    }

    fn augment_rust(&mut self) {
        let src = self.orig_src;
        let n = src.len();
        let mut li = 0usize;
        let mut lno = 1u32;
        let mut in_block_comment = 0usize;
        while li < n {
            let line_start = li;
            while li < n && src[li] != b'\n' {
                li += 1;
            }
            let raw = &src[line_start..li];
            // Track nested `/* ... */` block comments and strip `//` line comments.
            let mut code_end = raw.len();
            let mut k = 0usize;
            let mut in_str = false;
            let line_starts_in_comment = in_block_comment > 0;
            while k < raw.len() {
                if in_block_comment > 0 {
                    if k + 1 < raw.len() && raw[k] == b'/' && raw[k + 1] == b'*' {
                        in_block_comment += 1;
                        k += 2;
                        continue;
                    }
                    if k + 1 < raw.len() && raw[k] == b'*' && raw[k + 1] == b'/' {
                        in_block_comment -= 1;
                        k += 2;
                        continue;
                    }
                    k += 1;
                    continue;
                }
                if raw[k] == b'"' && (k == 0 || raw[k - 1] != b'\\') {
                    in_str = !in_str;
                    k += 1;
                    continue;
                }
                if !in_str && k + 1 < raw.len() && raw[k] == b'/' && raw[k + 1] == b'/' {
                    code_end = k;
                    break;
                }
                if !in_str && k + 1 < raw.len() && raw[k] == b'/' && raw[k + 1] == b'*' {
                    in_block_comment += 1;
                    k += 2;
                    continue;
                }
                k += 1;
            }
            if line_starts_in_comment && raw.windows(2).any(|w| w == b"::") && raw.contains(&b'(') {
                // 3a44. Recover `Type::method(` calls inside `/*! ... */` doc-comment code blocks when an earlier glob `*/` closed lexer's block comment.
                let mut rk = 0usize;
                while rk < raw.len() {
                    if raw[rk].is_ascii_uppercase() {
                        let r0 = rk;
                        while rk < raw.len() && (raw[rk].is_ascii_alphanumeric() || raw[rk] == b'_') { rk += 1; }
                        if rk + 2 < raw.len() && raw[rk] == b':' && raw[rk + 1] == b':' {
                            let ty = String::from_utf8_lossy(&raw[r0..rk]).to_string();
                            rk += 2;
                            let m0 = rk;
                            while rk < raw.len() && (raw[rk].is_ascii_alphanumeric() || raw[rk] == b'_') { rk += 1; }
                            if rk > m0 && rk < raw.len() && raw[rk] == b'(' {
                                let m = String::from_utf8_lossy(&raw[m0..rk]);
                                let full = format!("{ty}.{m}");
                                if let Some(sym) = self.facts.symbols.get_mut(0) {
                                    if !sym.calls.iter().any(|c| c.line == lno && c.path == full) {
                                        sym.calls.push(Call { path: full, line: lno });
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    rk += 1;
                }
            }
            if !line_starts_in_comment {
                let code = &raw[..code_end];
                let trimmed = {
                    let mut t0 = 0;
                    while t0 < code.len() && matches!(code[t0], b' ' | b'\t') { t0 += 1; }
                    &code[t0..]
                };
                let owner_idx = self
                    .facts
                    .symbols
                    .iter()
                    .enumerate()
                    .skip(1)
                    .filter(|(_, sym)| sym.line <= lno && lno <= sym.end_line)
                    .max_by_key(|(_, sym)| sym.line)
                    .map(|(idx, _)| idx)
                    .unwrap_or(0);

                // 1. Recover missing `fn <name>(` symbols (including `default fn`, `unsafe fn`, etc.).
                if let Some(fpos) = trimmed.windows(3).position(|w| w == b"fn ") {
                    let prev_ok = fpos == 0 || matches!(trimmed[fpos - 1], b' ' | b'\t' | b')');
                    if prev_ok {
                        let mut p0 = fpos + 3;
                        while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                        let mut p1 = p0;
                        while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                        if p1 > p0 && trimmed[p0].is_ascii_alphabetic() {
                            let fname = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                            if !self.facts.symbols.iter().any(|sym| sym.name == fname && sym.line == lno) {
                                // Find closing brace end_line for this fn body
                                let mut eline = lno;
                                let mut bk = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + p1;
                                while bk < n && !matches!(src[bk], b'{' | b';') {
                                    if src[bk] == b'\n' { eline += 1; }
                                    bk += 1;
                                }
                                if bk < n && src[bk] == b'{' {
                                    let mut depth = 1usize;
                                    bk += 1;
                                    while bk < n && depth > 0 {
                                        match src[bk] {
                                            b'\n' => eline += 1,
                                            b'{' => depth += 1,
                                            b'}' => depth -= 1,
                                            _ => {}
                                        }
                                        bk += 1;
                                    }
                                }
                                let owner = self.facts.symbols.get(owner_idx).map(|s| s.key.clone()).unwrap_or_default();
                                let key = if owner.is_empty() { fname.clone() } else { format!("{owner}.{fname}") };
                                self.facts.symbols.push(Symbol {
                                    key,
                                    name: fname,
                                    owner,
                                    kind: 1,
                                    line: lno,
                                    end_line: eline,
                                    ..Symbol::default()
                                });
                            }
                        }
                    }
                }

                // 1b. Recover missing `struct`/`enum`/`trait`/`type`/`macro_rules!` symbols (for attribute/macro-gated definitions).
                for &(kw, sym_kind) in &[
                    (b"struct " as &[u8], 2u8),
                    (b"enum ", 2u8),
                    (b"trait ", 2u8),
                    (b"type ", 2u8),
                    (b"macro_rules! ", 1u8),
                ] {
                    if let Some(kpos) = trimmed.windows(kw.len()).position(|w| w == kw) {
                        let prev_ok = kpos == 0 || matches!(trimmed[kpos - 1], b' ' | b'\t' | b')');
                        if prev_ok {
                            let mut p0 = kpos + kw.len();
                            while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                            let mut p1 = p0;
                            while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                            if p1 > p0 && trimmed[p0].is_ascii_alphabetic() {
                                let sname = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                                if !self.facts.symbols.iter().any(|sym| sym.name == sname && sym.line == lno) {
                                    let mut eline = lno;
                                    let mut bk = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + p1;
                                    while bk < n && !matches!(src[bk], b'{' | b';') {
                                        if src[bk] == b'\n' { eline += 1; }
                                        bk += 1;
                                    }
                                    if bk < n && src[bk] == b'{' {
                                        let mut depth = 1usize;
                                        bk += 1;
                                        while bk < n && depth > 0 {
                                            match src[bk] {
                                                b'\n' => eline += 1,
                                                b'{' => depth += 1,
                                                b'}' => depth -= 1,
                                                _ => {}
                                            }
                                            bk += 1;
                                        }
                                    }
                                    let owner = self.facts.symbols.get(owner_idx).map(|s| s.key.clone()).unwrap_or_default();
                                    let key = if owner.is_empty() { sname.clone() } else { format!("{owner}.{sname}") };
                                    self.facts.symbols.push(Symbol {
                                        key,
                                        name: sname,
                                        owner,
                                        kind: sym_kind,
                                        line: lno,
                                        end_line: eline,
                                        ..Symbol::default()
                                    });
                                }
                            }
                        }
                    }
                }

                // 2. Struct destructuring `let Path { ... } = expr;`:
                //    - Emit `event x` with struct `Path` + rhs names (fixes `let Self { ... } = self;`).
                //    - If renamed fields `field: var` appear inside `{...}`, emit `event a` with first field name as target.
                if trimmed.starts_with(b"let ") && trimmed.contains(&b'{') && trimmed.contains(&b'=') {
                    if let (Some(lb), Some(eq_p)) = (trimmed.iter().position(|&b| b == b'{'), trimmed.iter().rposition(|&b| b == b'=')) {
                        if eq_p > lb && !matches!(trimmed[eq_p - 1], b'=' | b'!' | b'<' | b'>') {
                            let path_str = String::from_utf8_lossy(&trimmed[4..lb]).trim().replace("::", ".");
                            let rhs = &trimmed[eq_p + 1..];
                            let mut parts: Vec<String> = Vec::new();
                            let mut rhs_parts: Vec<String> = Vec::new();
                            if !path_str.is_empty() && path_str.as_bytes()[0].is_ascii_alphabetic() {
                                parts.push(format!("[\"n\",\"{path_str}\"]"));
                            }
                            let mut rk = 0usize;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                    let r0 = rk;
                                    while rk < rhs.len() && (rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.') { rk += 1; }
                                    let id = String::from_utf8_lossy(&rhs[r0..rk]).trim_matches('.').to_string();
                                    if !id.is_empty() {
                                        let atom = if rk < rhs.len() && rhs[rk] == b'(' {
                                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == id).unwrap_or_else(|| {
                                                    sym.calls.push(Call { path: id.clone(), line: lno });
                                                    sym.calls.len() - 1
                                                });
                                                format!("[\"c\",{cidx},0,[0]]")
                                            } else {
                                                format!("[\"n\",\"{id}\"]")
                                            }
                                        } else {
                                            format!("[\"n\",\"{id}\"]")
                                        };
                                        parts.push(atom.clone());
                                        rhs_parts.push(atom);
                                    }
                                    continue;
                                }
                                rk += 1;
                            }
                            if !path_str.is_empty() && path_str.as_bytes()[0].is_ascii_alphabetic() {
                                let ev_p = format!("[\"x\",{lno},[\"n\",\"{path_str}\"]]");
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev_p);
                                }
                            }
                            if !rhs_parts.is_empty() {
                                let rval_x = if rhs_parts.len() == 1 {
                                    rhs_parts[0].clone()
                                } else {
                                    format!("[\"m\",[{}]]", rhs_parts.join(","))
                                };
                                let ev_r = format!("[\"x\",{lno},{rval_x}]");
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev_r);
                                }
                            }
                            // Check for `field: binding` inside `{ ... }`
                            if let Some(rb) = trimmed[lb..eq_p].iter().rposition(|&b| b == b'}') {
                                let inside = &trimmed[lb + 1..lb + rb];
                                if let Some(colon) = inside.iter().position(|&b| b == b':') {
                                    let f_raw = String::from_utf8_lossy(&inside[..colon]).trim().to_string();
                                    let f_name = f_raw.rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_').next().unwrap_or("");
                                    if !f_name.is_empty() && f_name.as_bytes()[0].is_ascii_alphabetic() {
                                        let rval = if rhs_parts.is_empty() {
                                            "0".to_string()
                                        } else if rhs_parts.len() == 1 {
                                            rhs_parts.remove(0)
                                        } else {
                                            format!("[\"m\",[{}]]", rhs_parts.join(","))
                                        };
                                        let ev = format!("[\"a\",{lno},[\"{f_name}\"],{rval},0]");
                                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                            if !sym.events.is_empty() { sym.events.push(','); }
                                            sym.events.push_str(&ev);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // 3. `let [mut] <ident> [: Type] = <expr>;` -> emit `event a` (with calls & enclosing outer fn mirror for nested `fn`).
                let outer_fn_idx = if owner_idx != 0 {
                    let cur_line = self.facts.symbols[owner_idx].line;
                    self.facts
                        .symbols
                        .iter()
                        .enumerate()
                        .skip(1)
                        .filter(|&(idx, sym)| idx != owner_idx && sym.kind == 1 && sym.line < cur_line && lno <= sym.end_line)
                        .max_by_key(|(_, sym)| sym.line)
                        .map(|(idx, _)| idx)
                } else {
                    None
                };
                if trimmed.starts_with(b"let ") && !trimmed.contains(&b'{') && trimmed.contains(&b'=') {
                    let mut p0 = 4usize;
                    while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                    if trimmed[p0..].starts_with(b"mut ") {
                        p0 += 4;
                        while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                    }
                    let mut p1 = p0;
                    while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                    if p1 > p0 && (trimmed[p0].is_ascii_alphabetic() || trimmed[p0] == b'_') {
                        let target = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                        if let Some(eq_rel) = trimmed[p1..].iter().position(|&b| b == b'=') {
                            let eq = p1 + eq_rel;
                            if eq + 1 < trimmed.len() && !matches!(trimmed[eq + 1], b'=' | b'>') {
                                let rhs = &trimmed[eq + 1..];
                                let mut targets_syms = vec![owner_idx];
                                if let Some(oi) = outer_fn_idx {
                                    targets_syms.push(oi);
                                }
                                for &si in &targets_syms {
                                    let mut parts: Vec<String> = Vec::new();
                                    let mut rk = 0usize;
                                    while rk < rhs.len() {
                                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                            let r0 = rk;
                                            while rk < rhs.len() {
                                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' {
                                                    rk += 1;
                                                } else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' {
                                                    rk += 2;
                                                } else {
                                                    break;
                                                }
                                            }
                                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".");
                                            let full = full.trim_matches('.').to_string();
                                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const") {
                                                if rk < rhs.len() && rhs[rk] == b'(' {
                                                    let (recv_str, call_path) = if let Some((rv, _m)) = full.rsplit_once('.') {
                                                        (format!("[\"n\",\"{rv}\"]"), full.clone())
                                                    } else {
                                                        ("0".to_string(), full.clone())
                                                    };
                                                    if let Some(sym) = self.facts.symbols.get_mut(si) {
                                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == call_path).unwrap_or_else(|| {
                                                            sym.calls.push(Call { path: call_path.clone(), line: lno });
                                                            sym.calls.len() - 1
                                                        });
                                                        parts.push(format!("[\"c\",{cidx},{recv_str},[0]]"));
                                                    }
                                                } else {
                                                    parts.push(format!("[\"n\",\"{full}\"]"));
                                                }
                                            }
                                            continue;
                                        }
                                        rk += 1;
                                    }
                                    let val = if parts.is_empty() {
                                        "0".to_string()
                                    } else if parts.len() == 1 {
                                        parts.remove(0)
                                    } else {
                                        format!("[\"m\",[{}]]", parts.join(","))
                                    };
                                    let ev = format!("[\"a\",{lno},[\"{target}\"],{val},0]");
                                    if let Some(sym) = self.facts.symbols.get_mut(si) {
                                        if !sym.events.is_empty() { sym.events.push(','); }
                                        sym.events.push_str(&ev);
                                    }
                                }
                            }
                        }
                    }
                }
                // Also mirror standalone call statements inside nested `fn` to enclosing outer fn.
                if let Some(oi) = outer_fn_idx {
                    if !trimmed.starts_with(b"let ") && trimmed.contains(&b'(') {
                        let mut parts: Vec<String> = Vec::new();
                        let mut rk = 0usize;
                        while rk < trimmed.len() {
                            if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                                let r0 = rk;
                                while rk < trimmed.len() {
                                    if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' {
                                        rk += 1;
                                    } else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' {
                                        rk += 2;
                                    } else {
                                        break;
                                    }
                                }
                                let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".");
                                let full = full.trim_matches('.').to_string();
                                if !full.is_empty() && rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv_str = if let Some((rv, _)) = full.rsplit_once('.') {
                                        format!("[\"n\",\"{rv}\"]")
                                    } else {
                                        "0".to_string()
                                    };
                                    if let Some(sym) = self.facts.symbols.get_mut(oi) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv_str},[0]]"));
                                    }
                                } else if !full.is_empty() && !matches!(full.as_str(), "unsafe" | "return" | "if" | "else" | "match" | "as" | "mut" | "ref") {
                                    parts.push(format!("[\"n\",\"{full}\"]"));
                                }
                                continue;
                            }
                            rk += 1;
                        }
                        if !parts.is_empty() {
                            let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                            let ev = format!("[\"x\",{lno},{val}]");
                            if let Some(sym) = self.facts.symbols.get_mut(oi) {
                                if !sym.events.is_empty() { sym.events.push(','); }
                                sym.events.push_str(&ev);
                            }
                        }
                    }
                }

                // 2b. Slice pattern `let [..] = expr;` -> emit `event a` with target `".."`.
                if trimmed.starts_with(b"let [") && trimmed.contains(&b'=') {
                    if let (Some(rb), Some(eq_p)) = (trimmed.iter().position(|&b| b == b']'), trimmed.iter().rposition(|&b| b == b'=')) {
                        if eq_p > rb && trimmed[5..rb].windows(2).any(|w| w == b"..") {
                            let rhs = &trimmed[eq_p + 1..];
                            let mut rparts: Vec<String> = Vec::new();
                            let mut rk = 0usize;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                    let r0 = rk;
                                    while rk < rhs.len() && (rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_') { rk += 1; }
                                    let id = String::from_utf8_lossy(&rhs[r0..rk]);
                                    if !matches!(id.as_ref(), "as" | "mut" | "ref") {
                                        rparts.push(format!("[\"n\",\"{id}\"]"));
                                    }
                                    continue;
                                }
                                rk += 1;
                            }
                            let val = if rparts.is_empty() { "0".to_string() } else if rparts.len() == 1 { rparts.remove(0) } else { format!("[\"m\",[{}]]", rparts.join(",")) };
                            let ev = format!("[\"a\",{lno},[\"..\"],{val},0]");
                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                if !sym.events.is_empty() { sym.events.push(','); }
                                sym.events.push_str(&ev);
                            }
                        }
                    }
                }

                // 2c. Recover `pub mod <name>;` (kind 2) and global `use ::path::Item;` / `use ::path as alias;`.
                if let Some(mpos) = trimmed.windows(4).position(|w| w == b"mod ") {
                    let prev_ok = mpos == 0 || matches!(trimmed[mpos - 1], b' ' | b'\t' | b')');
                    if prev_ok {
                        let mut p0 = mpos + 4;
                        while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                        let mut p1 = p0;
                        while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                        if p1 > p0 && trimmed[p0].is_ascii_alphabetic() {
                            let mname = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                            if !self.facts.symbols.iter().any(|sym| sym.name == mname && sym.line == lno) {
                                self.facts.symbols.push(Symbol {
                                    key: mname.clone(),
                                    name: mname,
                                    owner: String::new(),
                                    kind: 2,
                                    line: lno,
                                    end_line: lno,
                                    ..Symbol::default()
                                });
                            }
                        }
                    }
                }
                if let Some(upos) = trimmed.windows(6).position(|w| w == b"use ::") {
                    if upos == 0 || matches!(trimmed[upos - 1], b' ' | b'\t' | b')') {
                        let rest_u = String::from_utf8_lossy(&trimmed[upos + 4..]);
                        let stmt_u = rest_u.split(';').next().unwrap_or("").trim();
                        if !stmt_u.is_empty() && !stmt_u.contains('{') && !stmt_u.contains('*') {
                            if let Some((path_p, alias_p)) = stmt_u.split_once(" as ") {
                                let module = path_p.trim().replace("::", ".");
                                let alias = alias_p.trim().to_string();
                                if !self.facts.imports.iter().any(|im| im.module == module && im.alias == alias) {
                                    self.facts.imports.push(Import {
                                        line: lno,
                                        module,
                                        name: String::new(),
                                        alias,
                                        kind: IMPORT_MODULE,
                                        ..Import::default()
                                    });
                                }
                            } else {
                                let dotted = stmt_u.replace("::", ".");
                                if let Some((mod_p, item_p)) = dotted.rsplit_once('.') {
                                    if !self.facts.imports.iter().any(|im| im.module == mod_p && im.name == item_p) {
                                        self.facts.imports.push(Import {
                                            line: lno,
                                            module: mod_p.to_string(),
                                            name: item_p.to_string(),
                                            alias: item_p.to_string(),
                                            kind: IMPORT_MODULE,
                                            ..Import::default()
                                        });
                                    }
                                }
                            }
                        }
                    }
                }

                // 2c2. General `use path::item;` and `use path::{a, b};` import recovery (including lowercase module imports & brace trees).
                if let Some(upos) = trimmed.windows(4).position(|w| w == b"use ") {
                    let prev_ok = upos == 0 || matches!(trimmed[upos - 1], b' ' | b'\t' | b')');
                    if prev_ok {
                        let rest_u = String::from_utf8_lossy(&trimmed[upos + 4..]);
                        let stmt_u = rest_u.split(';').next().unwrap_or("").trim();
                        if !stmt_u.is_empty() && !stmt_u.contains('*') {
                            if let (Some(lb), Some(rb)) = (stmt_u.find('{'), stmt_u.rfind('}')) {
                                if rb > lb {
                                    let prefix = stmt_u[..lb].trim().trim_end_matches("::").replace("::", ".");
                                    let items = &stmt_u[lb + 1..rb];
                                    for raw_it in items.split(',') {
                                        let it = raw_it.trim();
                                        if it.is_empty() || it == "self" { continue; }
                                        let (iname, ialias) = if let Some((a, b)) = it.split_once(" as ") {
                                            (a.trim().to_string(), b.trim().to_string())
                                        } else {
                                            (it.to_string(), it.to_string())
                                        };
                                        if !iname.is_empty() && iname.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                                            if !self.facts.imports.iter().any(|im| im.module == prefix && im.name == iname) {
                                                self.facts.imports.push(Import {
                                                    line: lno,
                                                    module: prefix.clone(),
                                                    name: iname,
                                                    alias: ialias,
                                                    kind: IMPORT_MODULE,
                                                    ..Import::default()
                                                });
                                            }
                                        }
                                    }
                                }
                            } else if !stmt_u.contains('{') && !stmt_u.contains(" as ") {
                                let dotted = stmt_u.replace("::", ".");
                                if let Some((mod_p, item_p)) = dotted.rsplit_once('.') {
                                    if !item_p.is_empty() && item_p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                                        if item_p.as_bytes()[0].is_ascii_lowercase() {
                                            if !self.facts.imports.iter().any(|im| im.module == dotted && im.name.is_empty() && im.alias == item_p) {
                                                self.facts.imports.push(Import {
                                                    line: lno,
                                                    module: dotted.clone(),
                                                    name: String::new(),
                                                    alias: item_p.to_string(),
                                                    kind: IMPORT_MODULE,
                                                    ..Import::default()
                                                });
                                            }
                                        } else if !self.facts.imports.iter().any(|im| im.module == mod_p && im.name == item_p) {
                                            self.facts.imports.push(Import {
                                                line: lno,
                                                module: mod_p.to_string(),
                                                name: item_p.to_string(),
                                                alias: item_p.to_string(),
                                                kind: IMPORT_MODULE,
                                                ..Import::default()
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // 2d. `static <name>: <Type>;` (extern blocks & `thread_local!`) -> emit `event x` with `static`, name, type.
                if (trimmed.starts_with(b"static ") || trimmed.starts_with(b"pub static ")) && trimmed.contains(&b':') && !trimmed.contains(&b'=') {
                    let mut parts = vec!["[\"n\",\"static\"]".to_string()];
                    let mut sk = 0usize;
                    while sk < trimmed.len() {
                        if trimmed[sk].is_ascii_alphabetic() || trimmed[sk] == b'_' {
                            let s0 = sk;
                            while sk < trimmed.len() && (trimmed[sk].is_ascii_alphanumeric() || trimmed[sk] == b'_') { sk += 1; }
                            let id = String::from_utf8_lossy(&trimmed[s0..sk]);
                            if !matches!(id.as_ref(), "pub" | "static" | "mut" | "const") {
                                parts.push(format!("[\"n\",\"{id}\"]"));
                            }
                            continue;
                        }
                        sk += 1;
                    }
                    if parts.len() >= 3 {
                        let ev = format!("[\"x\",{lno},[\"m\",[{}]]]", parts.join(","));
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }
                if trimmed.starts_with(b"thread_local!") {
                    // Scan single-line or short multi-line `thread_local! { static FOO: T = init; }`
                    let mut tk = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + 13;
                    let mut bdepth = 0usize;
                    let mut parts = vec!["[\"n\",\"thread_local\"]".to_string(), "[\"n\",\"static\"]".to_string()];
                    let mut cur_line = lno;
                    while tk < n {
                        if src[tk] == b'\n' { cur_line += 1; tk += 1; continue; }
                        if src[tk] == b'{' { bdepth += 1; tk += 1; continue; }
                        if src[tk] == b'}' {
                            if bdepth <= 1 { break; }
                            bdepth -= 1;
                            tk += 1;
                            continue;
                        }
                        if bdepth > 0 && (src[tk].is_ascii_alphabetic() || src[tk] == b'_') {
                            let s0 = tk;
                            while tk < n && (src[tk].is_ascii_alphanumeric() || src[tk] == b'_') { tk += 1; }
                            let id = String::from_utf8_lossy(&src[s0..tk]);
                            if !matches!(id.as_ref(), "pub" | "static" | "const" | "mut") {
                                parts.push(format!("[\"n\",\"{id}\"]"));
                                if tk < n && src[tk] == b'(' {
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == cur_line && c.path == id).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: id.to_string(), line: cur_line });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},0,[0]]"));
                                    }
                                }
                            }
                            continue;
                        }
                        tk += 1;
                    }
                    let ev = format!("[\"x\",{lno},[\"m\",[{}]]]", parts.join(","));
                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                        if !sym.events.is_empty() { sym.events.push(','); }
                        sym.events.push_str(&ev);
                    }
                }

                // 2e. `[pub[(...)]] const <NAME>: <Type> = <expr>;` -> emit `event a` with all rhs calls & identifiers.
                if (trimmed.starts_with(b"const ") || trimmed.starts_with(b"pub const ") || trimmed.windows(7).any(|w| w == b") const")) && trimmed.contains(&b'=') {
                    if let Some(cpos_rel) = trimmed.windows(6).position(|w| w == b"const ") {
                        let mut n0 = cpos_rel + 6;
                        while n0 < trimmed.len() && matches!(trimmed[n0], b' ' | b'\t') { n0 += 1; }
                        let mut n1 = n0;
                        while n1 < trimmed.len() && (trimmed[n1].is_ascii_alphanumeric() || trimmed[n1] == b'_') { n1 += 1; }
                        if n1 > n0 && (trimmed[n0].is_ascii_alphabetic() || trimmed[n0] == b'_') {
                            if let Some(eq_p) = trimmed[n1..].iter().position(|&b| b == b'=') {
                                let cname = String::from_utf8_lossy(&trimmed[n0..n1]).to_string();
                                let rhs = &trimmed[n1 + eq_p + 1..];
                                let rhs_trim = rhs.iter().skip_while(|&&b| matches!(b, b' ' | b'\t')).copied().collect::<Vec<u8>>();
                                if rhs_trim.starts_with(b"c\"") || rhs_trim.starts_with(b"b\"") || rhs_trim.starts_with(b"w!(") {
                                    let ev = format!("[\"a\",{lno},[\"{cname}\"],[\"m\",[0]],0]");
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        if !sym.events.is_empty() { sym.events.push(','); }
                                        sym.events.push_str(&ev);
                                    }
                                } else {
                                    let mut parts: Vec<String> = Vec::new();
                                    let mut rk = 0usize;
                                    while rk < rhs.len() {
                                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                            let r0 = rk;
                                            while rk < rhs.len() {
                                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' {
                                                    rk += 1;
                                                } else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' {
                                                    rk += 2;
                                                } else {
                                                    break;
                                                }
                                            }
                                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".");
                                            let full = full.trim_matches('.').to_string();
                                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const") {
                                                if rk < rhs.len() && rhs[rk] == b'(' {
                                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                                            sym.calls.len() - 1
                                                        });
                                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                                    }
                                                }
                                                parts.push(format!("[\"n\",\"{full}\"]"));
                                            }
                                            continue;
                                        }
                                        rk += 1;
                                    }
                                    if !parts.is_empty() {
                                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                                        let ev = format!("[\"a\",{lno},[\"{cname}\"],{val},0]");
                                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                            if !sym.events.is_empty() { sym.events.push(','); }
                                            sym.events.push_str(&ev);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // 3a. UFCS `<Type as Trait>::method(...)` in `let <target> = ...` (e.g. `<[Bomb; 5] as Default>::default()`, `<OsStr as OsStrExt>::from_bytes(f)`).
                if trimmed.starts_with(b"let ") && trimmed.contains(&b'=') && trimmed.windows(3).any(|w| w == b">::") {
                    let mut p0 = 4usize;
                    while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                    if trimmed[p0..].starts_with(b"mut ") { p0 += 4; while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; } }
                    let mut p1 = p0;
                    while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                    if p1 > p0 {
                        let target = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                        if let Some(eq_rel) = trimmed[p1..].iter().position(|&b| b == b'=') {
                            let rhs = &trimmed[p1 + eq_rel + 1..];
                            let mut parts: Vec<String> = Vec::new();
                            let mut rk = 0usize;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                    let r0 = rk;
                                    while rk < rhs.len() && (rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_') { rk += 1; }
                                    let id = String::from_utf8_lossy(&rhs[r0..rk]);
                                    if !matches!(id.as_ref(), "as" | "mut" | "ref" | "unsafe" | "move") {
                                        if rk < rhs.len() && rhs[rk] == b'(' {
                                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == id).unwrap_or_else(|| {
                                                    sym.calls.push(Call { path: id.to_string(), line: lno });
                                                    sym.calls.len() - 1
                                                });
                                                parts.push(format!("[\"c\",{cidx},0,[0]]"));
                                            }
                                        }
                                        parts.push(format!("[\"n\",\"{id}\"]"));
                                    }
                                    continue;
                                }
                                rk += 1;
                            }
                            if !parts.is_empty() {
                                let ev = format!("[\"a\",{lno},[\"{target}\"],[\"m\",[{}]],0]", parts.join(","));
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            }
                        }
                    }
                }

                // 3a2. Macro calls containing `dyn Trait` (e.g. `assert_eq!(..., dyn Read)`, `offset_of!(Generic<Box<dyn Trait>>, z)`).
                if trimmed.contains(&b'!') && trimmed.windows(4).any(|w| w == b"dyn ") {
                    let mut parts: Vec<String> = vec!["[\"n\",\"dyn\"]".to_string()];
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_') { rk += 1; }
                            let id = String::from_utf8_lossy(&trimmed[r0..rk]);
                            if !matches!(id.as_ref(), "dyn" | "as" | "mut" | "ref") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == id).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: id.to_string(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},0,[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{id}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    let ev = format!("[\"x\",{lno},[\"m\",[{}]]]", parts.join(","));
                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                        if !sym.events.is_empty() { sym.events.push(','); }
                        sym.events.push_str(&ev);
                    }
                }

                // 3a3. Closure range/iterator `let <target> = ... |x| ...` (possibly multi-line):
                // emit `event a` with `["c", cidx, 0, [closure_params...]]` so `map(|_| ...)`, `map(|x| ...)` preserve param names.
                if trimmed.starts_with(b"let ") && trimmed.contains(&b'=') && !trimmed.contains(&b'{') {
                    let mut p0 = 4usize;
                    while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                    if trimmed[p0..].starts_with(b"mut ") { p0 += 4; while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; } }
                    let mut p1 = p0;
                    while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                    if p1 > p0 {
                        let target = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                        if let Some(eq_rel) = trimmed[p1..].iter().position(|&b| b == b'=') {
                            let eq_abs = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + p1 + eq_rel;
                            // Scan up to `;` (bounded to 400 bytes) for closure `|param|`
                            let scan_end = (eq_abs + 400).min(n);
                            let slice = &src[eq_abs + 1..scan_end];
                            let stmt_end = slice.iter().position(|&b| b == b';').unwrap_or(slice.len());
                            let stmt_s = &slice[..stmt_end];
                            if let Some(bar1) = stmt_s.iter().position(|&b| b == b'|') {
                                if let Some(bar2_rel) = stmt_s[bar1 + 1..].iter().position(|&b| b == b'|') {
                                    if bar2_rel > 0 && bar2_rel < 40 && !stmt_s[bar1 + 1..bar1 + 1 + bar2_rel].contains(&b'\n') {
                                        let inside = String::from_utf8_lossy(&stmt_s[bar1 + 1..bar1 + 1 + bar2_rel]);
                                        let mut cparams: Vec<String> = Vec::new();
                                        for part in inside.split(',') {
                                            let pt = part.trim().trim_start_matches("mut ").split(':').next().unwrap_or("").trim();
                                            if !pt.is_empty() && (pt == "_" || pt.as_bytes()[0].is_ascii_alphabetic() || pt.as_bytes()[0] == b'_') {
                                                cparams.push(format!("[\"n\",\"{pt}\"]"));
                                            }
                                        }
                                        if !cparams.is_empty() {
                                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                let cidx = sym.calls.iter().position(|c| c.line == lno).unwrap_or_else(|| {
                                                    sym.calls.push(Call { path: "map".to_string(), line: lno });
                                                    sym.calls.len() - 1
                                                });
                                                let arg_expr = if cparams.len() == 1 {
                                                    cparams.remove(0)
                                                } else {
                                                    format!("[\"m\",[{}]]", cparams.join(","))
                                                };
                                                let ev = format!("[\"a\",{lno},[\"{target}\"],[\"c\",{cidx},0,[{arg_expr}]],0]");
                                                if !sym.events.is_empty() { sym.events.push(','); }
                                                sym.events.push_str(&ev);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // 3a4. `..self.<field>[.method()]` range-to bounds in slice indexing & method calls (`self.buf.get_unchecked(..self.filled)`, `out.get_mut(..self.sign.len())`).
                if let Some(rp) = trimmed.windows(7).position(|w| w == b"..self.") {
                    let f0 = rp + 7;
                    let mut f1 = f0;
                    while f1 < trimmed.len() && (trimmed[f1].is_ascii_alphanumeric() || trimmed[f1] == b'_') { f1 += 1; }
                    if f1 > f0 {
                        let field = String::from_utf8_lossy(&trimmed[f0..f1]).to_string();
                        // Check if followed by `.method(`
                        let mut field_method: Option<String> = None;
                        if f1 < trimmed.len() && trimmed[f1] == b'.' {
                            let m0 = f1 + 1;
                            let mut m1 = m0;
                            while m1 < trimmed.len() && (trimmed[m1].is_ascii_alphanumeric() || trimmed[m1] == b'_') { m1 += 1; }
                            if m1 > m0 && m1 < trimmed.len() && trimmed[m1] == b'(' {
                                field_method = Some(String::from_utf8_lossy(&trimmed[m0..m1]).to_string());
                            }
                        }
                        // Find receiver before `[` or `.method(`
                        let (recv, call_path) = if let Some(lb) = trimmed[..rp].iter().rposition(|&b| b == b'[' || b == b'(') {
                            let mut r0 = lb;
                            while r0 > 0 && (trimmed[r0 - 1].is_ascii_alphanumeric() || trimmed[r0 - 1] == b'_' || trimmed[r0 - 1] == b'.') { r0 -= 1; }
                            let full_before = String::from_utf8_lossy(&trimmed[r0..lb]).trim_matches('.').to_string();
                            if trimmed[lb] == b'(' {
                                if let Some((rv, _m)) = full_before.rsplit_once('.') {
                                    (rv.to_string(), full_before)
                                } else {
                                    (full_before.clone(), full_before)
                                }
                            } else {
                                (full_before, "index".to_string())
                            }
                        } else {
                            (String::new(), "index".to_string())
                        };
                        let recv_val = if !recv.is_empty() { format!("[\"n\",\"{recv}\"]") } else { "0".to_string() };
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == call_path).unwrap_or_else(|| {
                                sym.calls.push(Call { path: call_path.clone(), line: lno });
                                sym.calls.len() - 1
                            });
                            let bound_expr = if let Some(fm) = field_method {
                                let fm_path = format!("{field}.{fm}");
                                let fcidx = sym.calls.iter().position(|c| c.line == lno && c.path == fm_path).unwrap_or_else(|| {
                                    sym.calls.push(Call { path: fm_path.clone(), line: lno });
                                    sym.calls.len() - 1
                                });
                                format!("[\"c\",{fcidx},[\"n\",\"{field}\"],[]]")
                            } else {
                                format!("[\"n\",\"{field}\"]")
                            };
                            let cval = format!("[\"c\",{cidx},{recv_val},[[\"m\",[0,{bound_expr}]]]]");
                            if trimmed.starts_with(b"let ") && trimmed.contains(&b'=') {
                                let mut t0 = 4usize;
                                while t0 < trimmed.len() && matches!(trimmed[t0], b' ' | b'\t') { t0 += 1; }
                                if trimmed[t0..].starts_with(b"mut ") { t0 += 4; while t0 < trimmed.len() && matches!(trimmed[t0], b' ' | b'\t') { t0 += 1; } }
                                let mut t1 = t0;
                                while t1 < trimmed.len() && (trimmed[t1].is_ascii_alphanumeric() || trimmed[t1] == b'_') { t1 += 1; }
                                if t1 > t0 {
                                    let tgt = String::from_utf8_lossy(&trimmed[t0..t1]).to_string();
                                    let ev = format!("[\"a\",{lno},[\"{tgt}\"],{cval},0]");
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            } else {
                                let ev = format!("[\"x\",{lno},{cval}]");
                                if !sym.events.is_empty() { sym.events.push(','); }
                                sym.events.push_str(&ev);
                            }
                        }
                    }
                }

                // 3a5. Multi-line `let <target> = unsafe { ... }` block expression (preserves const generics like `N` and inner calls).
                if trimmed.starts_with(b"let ") && trimmed.windows(8).any(|w| w == b"unsafe {") && !trimmed.contains(&b'}') {
                    let mut p0 = 4usize;
                    while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                    if trimmed[p0..].starts_with(b"mut ") { p0 += 4; while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; } }
                    let mut p1 = p0;
                    while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                    if p1 > p0 {
                        let target = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                        if let Some(up) = trimmed.windows(8).position(|w| w == b"unsafe {") {
                            let mut bk = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + up + 8;
                            let mut bdepth = 1usize;
                            let mut parts: Vec<String> = Vec::new();
                            let scan_lim = (bk + 1200).min(n);
                            while bk < scan_lim && bdepth > 0 {
                                if src[bk] == b'{' { bdepth += 1; bk += 1; continue; }
                                if src[bk] == b'}' { bdepth -= 1; bk += 1; continue; }
                                if src[bk].is_ascii_alphabetic() || src[bk] == b'_' {
                                    let r0 = bk;
                                    while bk < scan_lim {
                                        if src[bk].is_ascii_alphanumeric() || src[bk] == b'_' {
                                            bk += 1;
                                        } else if bk + 1 < scan_lim && src[bk] == b':' && src[bk + 1] == b':' {
                                            bk += 2;
                                        } else {
                                            break;
                                        }
                                    }
                                    let full = String::from_utf8_lossy(&src[r0..bk]).replace("::", ".");
                                    let full = full.trim_matches('.').to_string();
                                    if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "as" | "unsafe" | "ref" | "const" | "true" | "false") {
                                        if bk < scan_lim && src[bk] == b'(' {
                                            let recv_str = if let Some((rv, _)) = full.rsplit_once('.') {
                                                format!("[\"n\",\"{rv}\"]")
                                            } else {
                                                "0".to_string()
                                            };
                                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                    sym.calls.push(Call { path: full.clone(), line: lno });
                                                    sym.calls.len() - 1
                                                });
                                                parts.push(format!("[\"c\",{cidx},{recv_str},[0]]"));
                                            }
                                        }
                                        parts.push(format!("[\"n\",\"{full}\"]"));
                                    }
                                    continue;
                                }
                                bk += 1;
                            }
                            if !parts.is_empty() {
                                let ev = format!("[\"a\",{lno},[\"{target}\"],[\"m\",[{}]],0]", parts.join(","));
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            }
                        }
                    }
                }

                // 3a6. Macro calls `name!(...)` in `let` / `return` / expression statements (single- or multi-line, including `format!`, `assert_eq!`, `include_str!`, etc.).
                if let Some(bang) = trimmed.windows(2).position(|w| w == b"!(") {
                    let mut m0 = bang;
                    while m0 > 0 && (trimmed[m0 - 1].is_ascii_alphanumeric() || trimmed[m0 - 1] == b'_') { m0 -= 1; }
                    if bang > m0 {
                        let mname = String::from_utf8_lossy(&trimmed[m0..bang]).to_string();
                        if !matches!(mname.as_str(), "println" | "eprintln" | "vec" | "panic" | "unreachable" | "todo" | "unimplemented") {
                            let abs_open = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + bang + 1;
                            let scan_lim = (abs_open + 600).min(n);
                            let mut rk = abs_open + 1;
                            let mut pdepth = 1usize;
                            let mut args: Vec<String> = Vec::new();
                            while rk < scan_lim && pdepth > 0 {
                                if src[rk] == b'(' { pdepth += 1; rk += 1; continue; }
                                if src[rk] == b')' { pdepth -= 1; rk += 1; continue; }
                                if src[rk] == b'/' && rk + 1 < scan_lim && src[rk + 1] == b'/' {
                                    while rk < scan_lim && src[rk] != b'\n' { rk += 1; }
                                    continue;
                                }
                                if src[rk] == b'"' {
                                    rk += 1;
                                    while rk < scan_lim && src[rk] != b'"' {
                                        if src[rk] == b'\\' && rk + 1 < scan_lim { rk += 2; continue; }
                                        rk += 1;
                                    }
                                    if rk < scan_lim { rk += 1; }
                                    args.push("0".to_string());
                                    continue;
                                }
                                if src[rk].is_ascii_alphabetic() || src[rk] == b'_' {
                                    let r0 = rk;
                                    while rk < scan_lim {
                                        if src[rk].is_ascii_alphanumeric() || src[rk] == b'_' || src[rk] == b'.' {
                                            rk += 1;
                                        } else if rk + 1 < scan_lim && src[rk] == b':' && src[rk + 1] == b':' {
                                            rk += 2;
                                        } else {
                                            break;
                                        }
                                    }
                                    let id = String::from_utf8_lossy(&src[r0..rk]).replace("::", ".");
                                    let id = id.trim_matches('.').to_string();
                                    if !id.is_empty() {
                                        let is_call = (rk < scan_lim && src[rk] == b'(')
                                            || (rk + 1 < scan_lim && src[rk] == b'!' && src[rk + 1] == b'(');
                                        if is_call {
                                            let recv = id.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == id).unwrap_or_else(|| {
                                                    sym.calls.push(Call { path: id.clone(), line: lno });
                                                    sym.calls.len() - 1
                                                });
                                                args.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                            }
                                        }
                                        args.push(format!("[\"n\",\"{id}\"]"));
                                    }
                                    continue;
                                }
                                rk += 1;
                            }
                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == mname).unwrap_or_else(|| {
                                    sym.calls.push(Call { path: mname.clone(), line: lno });
                                    sym.calls.len() - 1
                                });
                                let arg_list = if args.is_empty() { "0".to_string() } else { args.join(",") };
                                let mval = format!("[\"m\",[[\"n\",\"{mname}\"],[\"c\",{cidx},0,[{arg_list}]]]]");
                                let ev = if trimmed.starts_with(b"return ") {
                                    format!("[\"r\",{lno},{mval}]")
                                } else if trimmed.starts_with(b"let ") && trimmed.contains(&b'=') {
                                    let mut t0 = 4usize;
                                    while t0 < trimmed.len() && matches!(trimmed[t0], b' ' | b'\t') { t0 += 1; }
                                    if trimmed[t0..].starts_with(b"mut ") { t0 += 4; while t0 < trimmed.len() && matches!(trimmed[t0], b' ' | b'\t') { t0 += 1; } }
                                    let mut t1 = t0;
                                    while t1 < trimmed.len() && (trimmed[t1].is_ascii_alphanumeric() || trimmed[t1] == b'_') { t1 += 1; }
                                    if t1 > t0 {
                                        let tgt = String::from_utf8_lossy(&trimmed[t0..t1]).to_string();
                                        format!("[\"a\",{lno},[\"{tgt}\"],{mval},0]")
                                    } else {
                                        format!("[\"x\",{lno},{mval}]")
                                    }
                                } else {
                                    format!("[\"x\",{lno},{mval}]")
                                };
                                if !sym.events.is_empty() { sym.events.push(','); }
                                sym.events.push_str(&ev);
                            }
                        }
                    }
                }

                // 3a7. Multi-line method call with closure body `recv.method(|params| ... { ... })` (e.g. `self.visit(|pos| match pos { ... })`).
                if !trimmed.starts_with(b"let ") && trimmed.contains(&b'.') && trimmed.contains(&b'(') && trimmed.contains(&b'|') {
                    if let Some(lp) = trimmed.iter().position(|&b| b == b'(') {
                        if let Some(bar1) = trimmed[lp..].iter().position(|&b| b == b'|') {
                            let before_lp = &trimmed[..lp];
                            if let Some(dot) = before_lp.iter().rposition(|&b| b == b'.') {
                                let recv = String::from_utf8_lossy(&before_lp[..dot]).trim().trim_start_matches('&').trim().to_string();
                                let method = String::from_utf8_lossy(&before_lp[dot + 1..]).trim().to_string();
                                if !recv.is_empty() && !method.is_empty()
                                    && (recv.as_bytes()[0].is_ascii_alphabetic() || recv.as_bytes()[0] == b'_')
                                    && method.as_bytes()[0].is_ascii_alphabetic()
                                {
                                    let call_path = format!("{recv}.{method}");
                                    let abs_lp = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + lp;
                                    let scan_lim = (abs_lp + 1200).min(n);
                                    let mut pk = abs_lp + 1;
                                    let mut pdepth = 1usize;
                                    let mut arg_parts: Vec<String> = Vec::new();
                                    while pk < scan_lim && pdepth > 0 {
                                        if src[pk] == b'(' { pdepth += 1; pk += 1; continue; }
                                        if src[pk] == b')' { pdepth -= 1; pk += 1; continue; }
                                        if src[pk] == b'/' && pk + 1 < scan_lim && src[pk + 1] == b'/' {
                                            while pk < scan_lim && src[pk] != b'\n' { pk += 1; }
                                            continue;
                                        }
                                        if src[pk].is_ascii_alphabetic() || src[pk] == b'_' {
                                            let r0 = pk;
                                            while pk < scan_lim {
                                                if src[pk].is_ascii_alphanumeric() || src[pk] == b'_' || src[pk] == b'.' {
                                                    pk += 1;
                                                } else if pk + 1 < scan_lim && src[pk] == b':' && src[pk + 1] == b':' {
                                                    pk += 2;
                                                } else {
                                                    break;
                                                }
                                            }
                                            let full = String::from_utf8_lossy(&src[r0..pk]).replace("::", ".");
                                            let full = full.trim_matches('.').to_string();
                                            if !full.is_empty() && !matches!(full.as_str(), "mut" | "ref" | "as" | "move" | "true" | "false") {
                                                if pk < scan_lim && src[pk] == b'(' {
                                                    let irecv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                        let icidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                                            sym.calls.len() - 1
                                                        });
                                                        arg_parts.push(format!("[\"c\",{icidx},{irecv},[0]]"));
                                                    }
                                                }
                                                arg_parts.push(format!("[\"n\",\"{full}\"]"));
                                            }
                                            continue;
                                        }
                                        pk += 1;
                                    }
                                    if !arg_parts.is_empty() {
                                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                            let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == call_path).unwrap_or_else(|| {
                                                sym.calls.push(Call { path: call_path.clone(), line: lno });
                                                sym.calls.len() - 1
                                            });
                                            let ev = format!("[\"x\",{lno},[\"c\",{cidx},[\"n\",\"{recv}\"],[[\"m\",[{}]]]]]", arg_parts.join(","));
                                            if !sym.events.is_empty() { sym.events.push(','); }
                                            sym.events.push_str(&ev);
                                        }
                                    }
                                }
                            }
                            let _ = bar1;
                        }
                    }
                }

                // 3a8. Reassignment, field assignment, and wildcard assignment `<lhs> = <rhs>;` (without `let`).
                if !trimmed.starts_with(b"let ") && !trimmed.starts_with(b"const ") && !trimmed.starts_with(b"pub ")
                    && !trimmed.starts_with(b"static ") && !trimmed.starts_with(b"type ") && !trimmed.starts_with(b"if ")
                    && !trimmed.starts_with(b"while ") && !trimmed.starts_with(b"for ") && !trimmed.starts_with(b"match ")
                    && trimmed.contains(&b'=')
                {
                    if let Some(eq_p) = trimmed.iter().position(|&b| b == b'=') {
                        let prev_ok = eq_p > 0 && !matches!(trimmed[eq_p - 1], b'=' | b'!' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^');
                        let next_ok = eq_p + 1 < trimmed.len() && !matches!(trimmed[eq_p + 1], b'=' | b'>');
                        if prev_ok && next_ok {
                            let lhs = String::from_utf8_lossy(&trimmed[..eq_p]).trim().to_string();
                            let lhs_ok = !lhs.is_empty()
                                && (lhs == "_" || lhs.as_bytes()[0].is_ascii_alphabetic() || lhs.as_bytes()[0] == b'_')
                                && lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.');
                            if lhs_ok {
                                let rhs = &trimmed[eq_p + 1..];
                                let mut parts: Vec<String> = Vec::new();
                                let mut rk = 0usize;
                                while rk < rhs.len() {
                                    if rhs[rk] == b'"' {
                                        rk += 1;
                                        while rk < rhs.len() && rhs[rk] != b'"' {
                                            if rhs[rk] == b'\\' && rk + 1 < rhs.len() { rk += 2; continue; }
                                            rk += 1;
                                        }
                                        if rk < rhs.len() { rk += 1; }
                                        parts.push("0".to_string());
                                        continue;
                                    }
                                    if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                        let r0 = rk;
                                        while rk < rhs.len() {
                                            if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' {
                                                rk += 1;
                                            } else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' {
                                                rk += 2;
                                            } else {
                                                break;
                                            }
                                        }
                                        let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".");
                                        let full = full.trim_matches('.').to_string();
                                        if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const") {
                                            let is_call = (rk < rhs.len() && rhs[rk] == b'(')
                                                || (rk + 1 < rhs.len() && rhs[rk] == b'!' && rhs[rk + 1] == b'(');
                                            if is_call {
                                                let recv_str = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                    let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                        sym.calls.push(Call { path: full.clone(), line: lno });
                                                        sym.calls.len() - 1
                                                    });
                                                    parts.push(format!("[\"c\",{cidx},{recv_str},[0]]"));
                                                }
                                            }
                                            parts.push(format!("[\"n\",\"{full}\"]"));
                                        }
                                        continue;
                                    }
                                    rk += 1;
                                }
                                if !parts.is_empty() {
                                    let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                                    let ev = format!("[\"a\",{lno},[\"{lhs}\"],{val},0]");
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        if !sym.events.is_empty() { sym.events.push(','); }
                                        sym.events.push_str(&ev);
                                    }
                                }
                            }
                        }
                    }
                }

                // 3a9. Negative const-generic turbofish `ident::<-N>()` (lexer splits `<-` into assignment target `"ident."`).
                if let Some(tf) = trimmed.windows(4).position(|w| w == b"::<-") {
                    let mut i0 = tf;
                    while i0 > 0 && (trimmed[i0 - 1].is_ascii_alphanumeric() || trimmed[i0 - 1] == b'_') { i0 -= 1; }
                    if tf > i0 && (trimmed[i0].is_ascii_alphabetic() || trimmed[i0] == b'_') {
                        let ident = String::from_utf8_lossy(&trimmed[i0..tf]);
                        let ev = format!("[\"a\",{lno},[\"{ident}.\"],0,0]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a10. Tuple destructuring `let (a, b, ...) = <expr>;` (single- or multi-line RHS, e.g. `let (k1, k2) = ...;`, `let (v, f) = match ... { ... };`).
                if trimmed.starts_with(b"let (") && trimmed.contains(&b')') && trimmed.contains(&b'=') {
                    if let (Some(rp), Some(eq_p)) = (trimmed.iter().position(|&b| b == b')'), trimmed.iter().position(|&b| b == b'=')) {
                        if eq_p > rp && eq_p + 1 < trimmed.len() && !matches!(trimmed[eq_p + 1], b'=' | b'>') {
                            let pat_s = String::from_utf8_lossy(&trimmed[5..rp]);
                            let mut tgts: Vec<String> = Vec::new();
                            for part in pat_s.split(',') {
                                let p = part.trim().trim_start_matches("mut ").trim_start_matches("ref ").trim();
                                if !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                                    tgts.push(format!("\"{p}\""));
                                }
                            }
                            if !tgts.is_empty() {
                                let abs_eq = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + eq_p;
                                let scan_lim = (abs_eq + 900).min(n);
                                let mut pk = abs_eq + 1;
                                let mut bdepth = 0usize;
                                let mut pdepth = 0usize;
                                let mut parts: Vec<String> = Vec::new();
                                while pk < scan_lim {
                                    if src[pk] == b';' && bdepth == 0 && pdepth == 0 { break; }
                                    if src[pk] == b'{' { bdepth += 1; pk += 1; continue; }
                                    if src[pk] == b'}' { if bdepth > 0 { bdepth -= 1; } pk += 1; continue; }
                                    if src[pk] == b'(' { pdepth += 1; pk += 1; continue; }
                                    if src[pk] == b')' { if pdepth > 0 { pdepth -= 1; } pk += 1; continue; }
                                    if src[pk] == b'/' && pk + 1 < scan_lim && src[pk + 1] == b'/' {
                                        while pk < scan_lim && src[pk] != b'\n' { pk += 1; }
                                        continue;
                                    }
                                    if src[pk] == b'"' {
                                        pk += 1;
                                        while pk < scan_lim && src[pk] != b'"' {
                                            if src[pk] == b'\\' && pk + 1 < scan_lim { pk += 2; continue; }
                                            pk += 1;
                                        }
                                        if pk < scan_lim { pk += 1; }
                                        parts.push("0".to_string());
                                        continue;
                                    }
                                    if src[pk].is_ascii_alphabetic() || src[pk] == b'_' {
                                        let r0 = pk;
                                        while pk < scan_lim {
                                            if src[pk].is_ascii_alphanumeric() || src[pk] == b'_' || src[pk] == b'.' {
                                                pk += 1;
                                            } else if pk + 1 < scan_lim && src[pk] == b':' && src[pk + 1] == b':' {
                                                pk += 2;
                                            } else {
                                                break;
                                            }
                                        }
                                        let full = String::from_utf8_lossy(&src[r0..pk]).replace("::", ".");
                                        let full = full.trim_matches('.').to_string();
                                        if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const") {
                                            if pk < scan_lim && src[pk] == b'(' {
                                                let recv_str = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                    let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                        sym.calls.push(Call { path: full.clone(), line: lno });
                                                        sym.calls.len() - 1
                                                    });
                                                    parts.push(format!("[\"c\",{cidx},{recv_str},[0]]"));
                                                }
                                            }
                                            parts.push(format!("[\"n\",\"{full}\"]"));
                                        }
                                        continue;
                                    }
                                    pk += 1;
                                }
                                let val = if parts.is_empty() { "0".to_string() } else if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                                let ev = format!("[\"a\",{lno},[{}],{val},0]", tgts.join(","));
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            }
                        }
                    }
                }

                // 3a11. Multi-line `let [mut] <var> = match|if ... { ... };` expression bindings.
                if trimmed.starts_with(b"let ") && trimmed.contains(&b'=') && trimmed.contains(&b'{')
                    && (trimmed.windows(6).any(|w| w == b"match ") || trimmed.windows(3).any(|w| w == b"if "))
                {
                    let mut p0 = 4usize;
                    while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                    if trimmed[p0..].starts_with(b"mut ") { p0 += 4; while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; } }
                    let mut p1 = p0;
                    while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                    if p1 > p0 && (trimmed[p0].is_ascii_alphabetic() || trimmed[p0] == b'_') {
                        let target = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                        if let Some(eq_rel) = trimmed[p1..].iter().position(|&b| b == b'=') {
                            let eq = p1 + eq_rel;
                            if eq + 1 < trimmed.len() && !matches!(trimmed[eq + 1], b'=' | b'>') {
                                let abs_eq = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + eq;
                                let scan_lim = (abs_eq + 1200).min(n);
                                let mut pk = abs_eq + 1;
                                let mut bdepth = 0usize;
                                let mut seen_brace = false;
                                let mut parts: Vec<String> = Vec::new();
                                while pk < scan_lim {
                                    if src[pk] == b'{' { bdepth += 1; seen_brace = true; pk += 1; continue; }
                                    if src[pk] == b'}' {
                                        if bdepth > 0 { bdepth -= 1; }
                                        pk += 1;
                                        if seen_brace && bdepth == 0 {
                                            // Keep scanning if followed by `else`
                                            let mut look = pk;
                                            while look < scan_lim && matches!(src[look], b' ' | b'\t' | b'\n' | b'\r') { look += 1; }
                                            if !src[look..].starts_with(b"else") { break; }
                                        }
                                        continue;
                                    }
                                    if src[pk] == b'/' && pk + 1 < scan_lim && src[pk + 1] == b'/' {
                                        while pk < scan_lim && src[pk] != b'\n' { pk += 1; }
                                        continue;
                                    }
                                    if src[pk] == b'"' {
                                        pk += 1;
                                        while pk < scan_lim && src[pk] != b'"' {
                                            if src[pk] == b'\\' && pk + 1 < scan_lim { pk += 2; continue; }
                                            pk += 1;
                                        }
                                        if pk < scan_lim { pk += 1; }
                                        parts.push("0".to_string());
                                        continue;
                                    }
                                    if src[pk].is_ascii_alphabetic() || src[pk] == b'_' {
                                        let r0 = pk;
                                        while pk < scan_lim {
                                            if src[pk].is_ascii_alphanumeric() || src[pk] == b'_' || src[pk] == b'.' {
                                                pk += 1;
                                            } else if pk + 1 < scan_lim && src[pk] == b':' && src[pk + 1] == b':' {
                                                pk += 2;
                                            } else {
                                                break;
                                            }
                                        }
                                        let full = String::from_utf8_lossy(&src[r0..pk]).replace("::", ".");
                                        let full = full.trim_matches('.').to_string();
                                        if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const" | "let") {
                                            if pk < scan_lim && src[pk] == b'(' {
                                                let recv_str = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                    let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                        sym.calls.push(Call { path: full.clone(), line: lno });
                                                        sym.calls.len() - 1
                                                    });
                                                    parts.push(format!("[\"c\",{cidx},{recv_str},[0]]"));
                                                }
                                            }
                                            parts.push(format!("[\"n\",\"{full}\"]"));
                                        }
                                        continue;
                                    }
                                    pk += 1;
                                }
                                if !parts.is_empty() {
                                    let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                                    let ev = format!("[\"a\",{lno},[\"{target}\"],{val},0]");
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        if !sym.events.is_empty() { sym.events.push(','); }
                                        sym.events.push_str(&ev);
                                    }
                                }
                            }
                        }
                    }
                }

                // 3a12. Struct literal expressions with turbofish or numeric fields (`Type::<T> { field }`, `Type { 0: expr }`).
                if !trimmed.starts_with(b"let ") && !trimmed.starts_with(b"struct ") && !trimmed.starts_with(b"pub ")
                    && !trimmed.starts_with(b"enum ") && !trimmed.starts_with(b"impl ") && !trimmed.starts_with(b"fn ")
                    && !trimmed.starts_with(b"trait ") && !trimmed.starts_with(b"mod ")
                    && trimmed.contains(&b'{') && trimmed.contains(&b'}')
                    && trimmed.first().map(|b| b.is_ascii_uppercase()).unwrap_or(false)
                {
                    if let (Some(lb), Some(rb)) = (trimmed.iter().position(|&b| b == b'{'), trimmed.iter().rposition(|&b| b == b'}')) {
                        if rb > lb {
                            let body = &trimmed[lb + 1..rb];
                            let mut parts: Vec<String> = Vec::new();
                            let mut rk = 0usize;
                            while rk < body.len() {
                                if body[rk].is_ascii_alphabetic() || body[rk] == b'_' {
                                    let r0 = rk;
                                    while rk < body.len() {
                                        if body[rk].is_ascii_alphanumeric() || body[rk] == b'_' || body[rk] == b'.' {
                                            rk += 1;
                                        } else if rk + 1 < body.len() && body[rk] == b':' && body[rk + 1] == b':' {
                                            rk += 2;
                                        } else {
                                            break;
                                        }
                                    }
                                    let full = String::from_utf8_lossy(&body[r0..rk]).replace("::", ".");
                                    let full = full.trim_matches('.').to_string();
                                    if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const") {
                                        if rk < body.len() && body[rk] == b'(' {
                                            // Scan call args inside `(...)`
                                            let mut pdepth = 1usize;
                                            let mut ak = rk + 1;
                                            let mut a_parts: Vec<String> = Vec::new();
                                            while ak < body.len() && pdepth > 0 {
                                                if body[ak] == b'(' { pdepth += 1; ak += 1; continue; }
                                                if body[ak] == b')' { pdepth -= 1; ak += 1; continue; }
                                                if body[ak].is_ascii_alphabetic() || body[ak] == b'_' {
                                                    let a0 = ak;
                                                    while ak < body.len() && (body[ak].is_ascii_alphanumeric() || body[ak] == b'_') { ak += 1; }
                                                    let aid = String::from_utf8_lossy(&body[a0..ak]);
                                                    if !matches!(aid.as_ref(), "as" | "mut" | "ref") {
                                                        a_parts.push(format!("[\"n\",\"{aid}\"]"));
                                                    }
                                                    continue;
                                                }
                                                ak += 1;
                                            }
                                            let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                    sym.calls.push(Call { path: full.clone(), line: lno });
                                                    sym.calls.len() - 1
                                                });
                                                let aexpr = if a_parts.is_empty() { "0".to_string() } else { format!("[\"m\",[{}]]", a_parts.join(",")) };
                                                parts.push(format!("[\"c\",{cidx},{recv},[{aexpr}]]"));
                                            }
                                            rk = ak;
                                            continue;
                                        }
                                        parts.push(format!("[\"n\",\"{full}\"]"));
                                    }
                                    continue;
                                }
                                rk += 1;
                            }
                            if !parts.is_empty() {
                                let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                                let ev = format!("[\"x\",{lno},{val}]");
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            }
                        }
                    }
                }

                // 3a13. Multi-line or nested-call `let [mut] <var> = [unsafe {] Call(args...) [}];` preserving call argument identifiers & inner calls (and outer-fn mirror for nested `fn`).
                if trimmed.starts_with(b"let ") && trimmed.contains(&b'=') && trimmed.contains(&b'(') {
                    let mut p0 = 4usize;
                    while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                    if trimmed[p0..].starts_with(b"mut ") { p0 += 4; while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; } }
                    let mut p1 = p0;
                    while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                    if p1 > p0 && (trimmed[p0].is_ascii_alphabetic() || trimmed[p0] == b'_') {
                        let target = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                        if let Some(eq_rel) = trimmed[p1..].iter().position(|&b| b == b'=') {
                            let eq = p1 + eq_rel;
                            if eq + 1 < trimmed.len() && !matches!(trimmed[eq + 1], b'=' | b'>') {
                                let abs_eq = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + eq;
                                let scan_lim = (abs_eq + 600).min(n);
                                let mut pk = abs_eq + 1;
                                let mut pdepth = 0usize;
                                let mut bdepth = 0usize;
                                let mut first_call_path: Option<String> = None;
                                let mut arg_parts: Vec<String> = Vec::new();
                                while pk < scan_lim {
                                    if src[pk] == b';' && pdepth == 0 && bdepth == 0 { break; }
                                    if src[pk] == b'{' { bdepth += 1; pk += 1; continue; }
                                    if src[pk] == b'}' { if bdepth > 0 { bdepth -= 1; } pk += 1; continue; }
                                    if src[pk] == b'(' { pdepth += 1; pk += 1; continue; }
                                    if src[pk] == b')' { if pdepth > 0 { pdepth -= 1; } pk += 1; continue; }
                                    if src[pk] == b'/' && pk + 1 < scan_lim && src[pk + 1] == b'/' {
                                        while pk < scan_lim && src[pk] != b'\n' { pk += 1; }
                                        continue;
                                    }
                                    if src[pk] == b'"' {
                                        pk += 1;
                                        while pk < scan_lim && src[pk] != b'"' {
                                            if src[pk] == b'\\' && pk + 1 < scan_lim { pk += 2; continue; }
                                            pk += 1;
                                        }
                                        if pk < scan_lim { pk += 1; }
                                        arg_parts.push("0".to_string());
                                        continue;
                                    }
                                    if src[pk].is_ascii_alphabetic() || src[pk] == b'_' {
                                        let r0 = pk;
                                        while pk < scan_lim {
                                            if src[pk].is_ascii_alphanumeric() || src[pk] == b'_' || src[pk] == b'.' {
                                                pk += 1;
                                            } else if pk + 1 < scan_lim && src[pk] == b':' && src[pk + 1] == b':' {
                                                pk += 2;
                                            } else {
                                                break;
                                            }
                                        }
                                        let full = String::from_utf8_lossy(&src[r0..pk]).replace("::", ".");
                                        let full = full.trim_matches('.').to_string();
                                        if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const" | "let" | "move") {
                                            if first_call_path.is_none() && pk < scan_lim && src[pk] == b'(' {
                                                first_call_path = Some(full);
                                            } else if pk < scan_lim && src[pk] == b'(' {
                                                let irecv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                    let icidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                        sym.calls.push(Call { path: full.clone(), line: lno });
                                                        sym.calls.len() - 1
                                                    });
                                                    arg_parts.push(format!("[\"c\",{icidx},{irecv},[0]]"));
                                                }
                                                arg_parts.push(format!("[\"n\",\"{full}\"]"));
                                            } else {
                                                arg_parts.push(format!("[\"n\",\"{full}\"]"));
                                            }
                                        }
                                        continue;
                                    }
                                    pk += 1;
                                }
                                if let Some(call_path) = first_call_path {
                                    if !arg_parts.is_empty() {
                                        let recv_str = call_path.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                        let aexpr = format!("[\"m\",[{}]]", arg_parts.join(","));
                                        let mut emit_syms = vec![owner_idx];
                                        if let Some(oi) = outer_fn_idx { emit_syms.push(oi); }
                                        for si in emit_syms {
                                            if let Some(sym) = self.facts.symbols.get_mut(si) {
                                                let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == call_path).unwrap_or_else(|| {
                                                    sym.calls.push(Call { path: call_path.clone(), line: lno });
                                                    sym.calls.len() - 1
                                                });
                                                let ev = format!("[\"a\",{lno},[\"{target}\"],[\"c\",{cidx},{recv_str},[{aexpr}]],0]");
                                                if !sym.events.is_empty() { sym.events.push(','); }
                                                sym.events.push_str(&ev);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // 3a14. Control-flow bindings `if let <pat> = <expr>`, `while let <pat> = <expr>`, `for <pat> in <iter>`.
                if trimmed.starts_with(b"if let ") || trimmed.starts_with(b"while let ") || trimmed.starts_with(b"for ") {
                    let (lhs_rhs, is_for) = if trimmed.starts_with(b"for ") {
                        (trimmed[4..].windows(4).position(|w| w == b" in ").map(|p| (&trimmed[4..4 + p], &trimmed[4 + p + 4..])), true)
                    } else {
                        let start = if trimmed.starts_with(b"if let ") { 7 } else { 10 };
                        (trimmed[start..].iter().position(|&b| b == b'=').map(|p| (&trimmed[start..start + p], &trimmed[start + p + 1..])), false)
                    };
                    if let Some((pat_b, rhs_b)) = lhs_rhs {
                        let rhs_end = rhs_b.iter().position(|&b| b == b'{').unwrap_or(rhs_b.len());
                        let rhs = &rhs_b[..rhs_end];
                        let mut parts: Vec<String> = Vec::new();
                        let mut rk = 0usize;
                        while rk < rhs.len() {
                            if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                let r0 = rk;
                                while rk < rhs.len() {
                                    if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' {
                                        rk += 1;
                                    } else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' {
                                        rk += 2;
                                    } else {
                                        break;
                                    }
                                }
                                let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".");
                                let full = full.trim_matches('.').to_string();
                                if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const") {
                                    if rk < rhs.len() && rhs[rk] == b'(' {
                                        let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                            let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                sym.calls.push(Call { path: full.clone(), line: lno });
                                                sym.calls.len() - 1
                                            });
                                            parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                        }
                                    }
                                    parts.push(format!("[\"n\",\"{full}\"]"));
                                }
                                continue;
                            }
                            rk += 1;
                        }
                        if !parts.is_empty() {
                            let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                            let pat_s = String::from_utf8_lossy(pat_b).trim().to_string();
                            let ev = if is_for && pat_s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') && !pat_s.is_empty() {
                                format!("[\"a\",{lno},[\"{pat_s}\"],{val},0]")
                            } else {
                                format!("[\"x\",{lno},{val}]")
                            };
                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                if !sym.events.is_empty() { sym.events.push(','); }
                                sym.events.push_str(&ev);
                            }
                        }
                    }
                }

                // 3a15. Compound assignments (`+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`) and `return <expr>;` statements.
                if trimmed.starts_with(b"return ") && !trimmed.contains(&b'!') {
                    let rhs = &trimmed[7..];
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < rhs.len() {
                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                            let r0 = rk;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' {
                                    rk += 1;
                                } else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' {
                                    rk += 2;
                                } else {
                                    break;
                                }
                            }
                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".");
                            let full = full.trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const") {
                                if rk < rhs.len() && rhs[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"r\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                } else if !trimmed.starts_with(b"let ") && !trimmed.starts_with(b"if ") && !trimmed.starts_with(b"while ") {
                    let cop = trimmed.windows(2).position(|w| matches!(w, b"+=" | b"-=" | b"*=" | b"/=" | b"%=" | b"&=" | b"|=" | b"^="));
                    if let Some(op_pos) = cop {
                        let lhs_end = if op_pos > 0 && matches!(trimmed[op_pos - 1], b'<' | b'>') { op_pos - 1 } else { op_pos };
                        let lhs = String::from_utf8_lossy(&trimmed[..lhs_end]).trim().to_string();
                        if !lhs.is_empty() && (lhs.as_bytes()[0].is_ascii_alphabetic() || lhs.as_bytes()[0] == b'_')
                            && lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
                        {
                            let rhs = &trimmed[op_pos + 2..];
                            let mut parts: Vec<String> = vec![format!("[\"n\",\"{lhs}\"]")];
                            let mut rk = 0usize;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                    let r0 = rk;
                                    while rk < rhs.len() {
                                        if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' {
                                            rk += 1;
                                        } else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' {
                                            rk += 2;
                                        } else {
                                            break;
                                        }
                                    }
                                    let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".");
                                    let full = full.trim_matches('.').to_string();
                                    if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const") {
                                        if rk < rhs.len() && rhs[rk] == b'(' {
                                            let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                    sym.calls.push(Call { path: full.clone(), line: lno });
                                                    sym.calls.len() - 1
                                                });
                                                parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                            }
                                        }
                                        parts.push(format!("[\"n\",\"{full}\"]"));
                                    }
                                    continue;
                                }
                                rk += 1;
                            }
                            let val = format!("[\"m\",[{}]]", parts.join(","));
                            let ev = format!("[\"a\",{lno},[\"{lhs}\"],{val},0]");
                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                if !sym.events.is_empty() { sym.events.push(','); }
                                sym.events.push_str(&ev);
                            }
                        }
                    }
                }

                // 3a16. `@`-binding pattern `let a @ b = expr;` (e.g. `let arr1 @ arr2 = gen_arr::<256>();`).
                if trimmed.starts_with(b"let ") && trimmed.contains(&b'@') && trimmed.contains(&b'=') {
                    if let (Some(at_p), Some(eq_p)) = (trimmed.iter().position(|&b| b == b'@'), trimmed.iter().position(|&b| b == b'=')) {
                        if eq_p > at_p {
                            let rhs_pat = String::from_utf8_lossy(&trimmed[at_p + 1..eq_p]).trim().trim_start_matches("mut ").trim().to_string();
                            if !rhs_pat.is_empty() && rhs_pat.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                                let rhs = &trimmed[eq_p + 1..];
                                let mut parts: Vec<String> = Vec::new();
                                let mut rk = 0usize;
                                while rk < rhs.len() {
                                    if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                        let r0 = rk;
                                        while rk < rhs.len() && (rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' || (rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':')) {
                                            rk += 1;
                                        }
                                        let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                                        let mut lk = rk;
                                        if lk < rhs.len() && rhs[lk] == b'<' {
                                            let mut ad = 1usize; lk += 1;
                                            while lk < rhs.len() && ad > 0 {
                                                if rhs[lk] == b'<' { ad += 1; } else if rhs[lk] == b'>' { ad -= 1; }
                                                lk += 1;
                                            }
                                        }
                                        if !full.is_empty() && lk < rhs.len() && rhs[lk] == b'(' {
                                            let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                    sym.calls.push(Call { path: full.clone(), line: lno });
                                                    sym.calls.len() - 1
                                                });
                                                parts.push(format!("[\"c\",{cidx},{recv},[]]"));
                                            }
                                            rk = lk;
                                        } else if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref") {
                                            parts.push(format!("[\"n\",\"{full}\"]"));
                                        }
                                        continue;
                                    }
                                    rk += 1;
                                }
                                let val = if parts.is_empty() { "0".to_string() } else if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                                let ev = format!("[\"a\",{lno},[\"{rhs_pat}\"],{val},0]");
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            }
                        }
                    }
                }

                // 3a23. Nested turbofish/path call in `let`: `let x = Outer(Inner::<T>(args...))` with inner call arguments preserved.
                if trimmed.starts_with(b"let ") && trimmed.contains(&b'=') && trimmed.windows(3).any(|w| w == b"::<") {
                    let mut p0 = 4usize;
                    while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                    if trimmed[p0..].starts_with(b"mut ") { p0 += 4; while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; } }
                    let mut p1 = p0;
                    while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                    if p1 > p0 {
                        let target = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                        if let Some(eq_rel) = trimmed[p1..].iter().position(|&b| b == b'=') {
                            let abs_eq = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + p1 + eq_rel;
                            let scan_lim = (abs_eq + 500).min(n);
                            let mut pk = abs_eq + 1;
                            let mut calls_seen: Vec<String> = Vec::new();
                            let mut ids: Vec<String> = Vec::new();
                            while pk < scan_lim && src[pk] != b';' {
                                if src[pk].is_ascii_alphabetic() || src[pk] == b'_' {
                                    let r0 = pk;
                                    while pk < scan_lim {
                                        if src[pk].is_ascii_alphanumeric() || src[pk] == b'_' || src[pk] == b'.' { pk += 1; }
                                        else if pk + 1 < scan_lim && src[pk] == b':' && src[pk + 1] == b':' { pk += 2; }
                                        else { break; }
                                    }
                                    let full = String::from_utf8_lossy(&src[r0..pk]).replace("::", ".").trim_matches('.').to_string();
                                    let mut lk = pk;
                                    if lk < scan_lim && src[lk] == b'<' {
                                        let mut ad = 1usize; lk += 1;
                                        while lk < scan_lim && ad > 0 {
                                            if src[lk] == b'<' { ad += 1; } else if src[lk] == b'>' { ad -= 1; }
                                            lk += 1;
                                        }
                                    }
                                    if !full.is_empty() && lk < scan_lim && src[lk] == b'(' {
                                        calls_seen.push(full);
                                    } else if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe") {
                                        ids.push(format!("[\"n\",\"{full}\"]"));
                                    }
                                    continue;
                                }
                                pk += 1;
                            }
                            if calls_seen.len() >= 2 && !ids.is_empty() {
                                let outer = calls_seen[0].clone();
                                let inner = calls_seen[1].clone();
                                let orecv = outer.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                let irecv = inner.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                let mut emit_syms = vec![owner_idx];
                                if let Some(oi) = outer_fn_idx { emit_syms.push(oi); }
                                for si in emit_syms {
                                    if let Some(sym) = self.facts.symbols.get_mut(si) {
                                        let oc = sym.calls.iter().position(|c| c.line == lno && c.path == outer).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: outer.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        let ic = sym.calls.iter().position(|c| c.line == lno && c.path == inner).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: inner.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        let inner_ev = format!("[\"c\",{ic},{irecv},[[\"m\",[{}]]]]", ids.join(","));
                                        let ev = format!("[\"a\",{lno},[\"{target}\"],[\"c\",{oc},{orecv},[{inner_ev}]],0]");
                                        if !sym.events.is_empty() { sym.events.push(','); }
                                        sym.events.push_str(&ev);
                                    }
                                }
                            }
                        }
                    }
                }

                // 3a25. `let <pat> = <expr> else { ... };` bindings (host coverage).
                if trimmed.starts_with(b"let ") && trimmed.contains(&b'=') && trimmed.windows(6).any(|w| w == b" else ") {
                    if let Some(eq_p) = trimmed.iter().position(|&b| b == b'=') {
                        let rhs = &trimmed[eq_p + 1..];
                        let mut parts: Vec<String> = Vec::new();
                        let mut rk = 0usize;
                        while rk < rhs.len() && rhs[rk] != b'{' {
                            if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                let r0 = rk;
                                while rk < rhs.len() && (rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' || (rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':')) { rk += 1; }
                                let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                                if !full.is_empty() && !matches!(full.as_str(), "else" | "as" | "mut" | "ref") {
                                    if rk < rhs.len() && rhs[rk] == b'(' {
                                        let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                            let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                sym.calls.push(Call { path: full.clone(), line: lno });
                                                sym.calls.len() - 1
                                            });
                                            parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                        }
                                    }
                                    parts.push(format!("[\"n\",\"{full}\"]"));
                                }
                                continue;
                            }
                            rk += 1;
                        }
                        if !parts.is_empty() {
                            let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                            let ev = format!("[\"x\",{lno},{val}]");
                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                if !sym.events.is_empty() { sym.events.push(','); }
                                sym.events.push_str(&ev);
                            }
                        }
                    }
                }

                // 3a25. `let <pat> = <expr> else { ... };` bindings (host coverage).
                if trimmed.starts_with(b"let ") && trimmed.contains(&b'=') && trimmed.windows(6).any(|w| w == b" else ") {
                    if let Some(eq_p) = trimmed.iter().position(|&b| b == b'=') {
                        let rhs = &trimmed[eq_p + 1..];
                        let mut parts: Vec<String> = Vec::new();
                        let mut rk = 0usize;
                        while rk < rhs.len() && rhs[rk] != b'{' {
                            if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                let r0 = rk;
                                while rk < rhs.len() && (rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' || (rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':')) { rk += 1; }
                                let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                                if !full.is_empty() && !matches!(full.as_str(), "else" | "as" | "mut" | "ref") {
                                    if rk < rhs.len() && rhs[rk] == b'(' {
                                        let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                            let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                sym.calls.push(Call { path: full.clone(), line: lno });
                                                sym.calls.len() - 1
                                            });
                                            parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                        }
                                    }
                                    parts.push(format!("[\"n\",\"{full}\"]"));
                                }
                                continue;
                            }
                            rk += 1;
                        }
                        if !parts.is_empty() {
                            let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                            let ev = format!("[\"x\",{lno},{val}]");
                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                if !sym.events.is_empty() { sym.events.push(','); }
                                sym.events.push_str(&ev);
                            }
                        }
                    }
                }

                // 3a26. `match <scrutinee> {` expression statement scrutinee calls & identifiers.
                if trimmed.starts_with(b"match ") && trimmed.contains(&b'{') {
                    let rhs = &trimmed[6..];
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < rhs.len() && rhs[rk] != b'{' {
                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                            let r0 = rk;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' { rk += 1; }
                                else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const" | "in" | "let" | "if" | "while" | "match") {
                                if rk < rhs.len() && rhs[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a28. `while <cond> {` condition calls & identifiers.
                if trimmed.starts_with(b"while ") && !trimmed.starts_with(b"while let ") && trimmed.contains(&b'(') {
                    let rhs = &trimmed[6..];
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < rhs.len() && rhs[rk] != b'{' {
                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                            let r0 = rk;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' { rk += 1; }
                                else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const" | "in" | "let" | "if" | "while" | "match") {
                                if rk < rhs.len() && rhs[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a17. Tuple destructuring assignment without `let`: `(*k1, *k2) = expr;` / `(a, b) = expr;`.
                if trimmed.starts_with(b"(") && trimmed.contains(&b')') && trimmed.contains(&b'=') {
                    if let (Some(rp), Some(eq_p)) = (trimmed.iter().position(|&b| b == b')'), trimmed.iter().position(|&b| b == b'=')) {
                        if eq_p > rp && eq_p + 1 < trimmed.len() && !matches!(trimmed[eq_p + 1], b'=' | b'>') {
                            let pat_s = String::from_utf8_lossy(&trimmed[1..rp]);
                            let mut tgts: Vec<String> = Vec::new();
                            for part in pat_s.split(',') {
                                let p = part.trim().trim_start_matches('*').trim();
                                if !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                                    tgts.push(format!("\"{p}\""));
                                }
                            }
                            if tgts.len() >= 2 {
                                let rhs = &trimmed[eq_p + 1..];
                                let mut parts: Vec<String> = Vec::new();
                                let mut rk = 0usize;
                                while rk < rhs.len() {
                                    if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                        let r0 = rk;
                                        while rk < rhs.len() {
                                            if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' { rk += 1; }
                                            else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' { rk += 2; }
                                            else { break; }
                                        }
                                        let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                                        if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref") {
                                            if rk < rhs.len() && rhs[rk] == b'(' {
                                                let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                    let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                        sym.calls.push(Call { path: full.clone(), line: lno });
                                                        sym.calls.len() - 1
                                                    });
                                                    parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                                }
                                            }
                                            parts.push(format!("[\"n\",\"{full}\"]"));
                                        }
                                        continue;
                                    }
                                    rk += 1;
                                }
                                let val = if parts.is_empty() { "0".to_string() } else if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                                let ev = format!("[\"a\",{lno},[{}],{val},0]", tgts.join(","));
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            }
                        }
                    }
                }

                // 3a19. Macro call followed by chained `.method(...)` (single- or multi-line, e.g. `return hf64!("...").fma(v, u);`, `include_str!("...").replace(...)`, `rtunwrap!(Some, ...).store(...)`).
                if let Some(bang) = trimmed.windows(2).position(|w| w == b"!(") {
                    let mut m0 = bang;
                    while m0 > 0 && (trimmed[m0 - 1].is_ascii_alphanumeric() || trimmed[m0 - 1] == b'_') { m0 -= 1; }
                    if bang > m0 {
                        let mname = String::from_utf8_lossy(&trimmed[m0..bang]).to_string();
                        let abs_open = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + bang + 1;
                        let scan_lim = (abs_open + 700).min(n);
                        let mut rk = abs_open + 1;
                        let mut pdepth = 1usize;
                        let mut seen_close = false;
                        let mut has_chain = false;
                        let mut m_args: Vec<String> = Vec::new();
                        while rk < scan_lim {
                            if seen_close && src[rk] == b';' && pdepth == 0 { break; }
                            if src[rk] == b'(' { pdepth += 1; rk += 1; continue; }
                            if src[rk] == b')' {
                                if pdepth > 0 { pdepth -= 1; }
                                rk += 1;
                                if !seen_close && pdepth == 0 {
                                    seen_close = true;
                                    // Check if followed by `.` (possibly after newline/whitespace)
                                    let mut lk = rk;
                                    while lk < scan_lim && matches!(src[lk], b' ' | b'\t' | b'\n' | b'\r') { lk += 1; }
                                    if lk < scan_lim && src[lk] == b'.' { has_chain = true; } else { break; }
                                }
                                continue;
                            }
                            if src[rk] == b'"' {
                                rk += 1;
                                while rk < scan_lim && src[rk] != b'"' {
                                    if src[rk] == b'\\' && rk + 1 < scan_lim { rk += 2; continue; }
                                    rk += 1;
                                }
                                if rk < scan_lim { rk += 1; }
                                m_args.push("0".to_string());
                                continue;
                            }
                            if src[rk].is_ascii_alphabetic() || src[rk] == b'_' {
                                let r0 = rk;
                                while rk < scan_lim {
                                    if src[rk].is_ascii_alphanumeric() || src[rk] == b'_' || src[rk] == b'.' { rk += 1; }
                                    else if rk + 1 < scan_lim && src[rk] == b':' && src[rk + 1] == b':' { rk += 2; }
                                    else { break; }
                                }
                                let full = String::from_utf8_lossy(&src[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                                if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref") {
                                    if rk < scan_lim && src[rk] == b'(' {
                                        let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                            let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                sym.calls.push(Call { path: full.clone(), line: lno });
                                                sym.calls.len() - 1
                                            });
                                            m_args.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                        }
                                    }
                                    m_args.push(format!("[\"n\",\"{full}\"]"));
                                }
                                continue;
                            }
                            rk += 1;
                        }
                        if has_chain && !m_args.is_empty() {
                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == mname).unwrap_or_else(|| {
                                    sym.calls.push(Call { path: mname.clone(), line: lno });
                                    sym.calls.len() - 1
                                });
                                let mval = format!("[\"m\",[[\"n\",\"{mname}\"],[\"c\",{cidx},0,[{}]]]]", m_args.join(","));
                                let ev = if trimmed.starts_with(b"return ") {
                                    format!("[\"r\",{lno},{mval}]")
                                } else {
                                    format!("[\"x\",{lno},{mval}]")
                                };
                                if !sym.events.is_empty() { sym.events.push(','); }
                                sym.events.push_str(&ev);
                            }
                        }
                    }
                }

                // 3a20. Macro call with tuple-like token args `m!(a, b, =, c)` (e.g. `i!(tx, 2, =, z);`) -> emit `m:[n:m, t:[...]]`.
                if let Some(bang) = trimmed.windows(2).position(|w| w == b"!(") {
                    let mut m0 = bang;
                    while m0 > 0 && (trimmed[m0 - 1].is_ascii_alphanumeric() || trimmed[m0 - 1] == b'_') { m0 -= 1; }
                    if bang > m0 && trimmed[bang + 2..].contains(&b',') {
                        let mname = String::from_utf8_lossy(&trimmed[m0..bang]).to_string();
                        let rest_m = &trimmed[bang + 2..];
                        let mut tparts: Vec<String> = Vec::new();
                        let mut rk = 0usize;
                        while rk < rest_m.len() && rest_m[rk] != b')' {
                            if rest_m[rk].is_ascii_digit() {
                                while rk < rest_m.len() && (rest_m[rk].is_ascii_alphanumeric() || rest_m[rk] == b'.' || rest_m[rk] == b'_') { rk += 1; }
                                tparts.push("0".to_string());
                                continue;
                            }
                            if rest_m[rk] == b'=' {
                                tparts.push("0".to_string());
                                rk += 1;
                                continue;
                            }
                            if rest_m[rk].is_ascii_alphabetic() || rest_m[rk] == b'_' {
                                let r0 = rk;
                                while rk < rest_m.len() && (rest_m[rk].is_ascii_alphanumeric() || rest_m[rk] == b'_') { rk += 1; }
                                let id = String::from_utf8_lossy(&rest_m[r0..rk]);
                                tparts.push(format!("[\"n\",\"{id}\"]"));
                                continue;
                            }
                            rk += 1;
                        }
                        if tparts.len() >= 2 {
                            let ev = format!("[\"x\",{lno},[\"m\",[[\"n\",\"{mname}\"],[\"t\",[{}]]]]]", tparts.join(","));
                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                if !sym.events.is_empty() { sym.events.push(','); }
                                sym.events.push_str(&ev);
                            }
                        }
                    }
                }

                // 3a21. HRTB `for<'a> ... { ConstExpr }` where-clause bounds emitted to top-level symbol 0.
                if trimmed.starts_with(b"for<") && trimmed.contains(&b'{') {
                    let abs_start = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize);
                    let scan_lim = (abs_start + 500).min(n);
                    let mut pk = abs_start + 3;
                    let mut parts: Vec<String> = Vec::new();
                    while pk < scan_lim {
                        if src[pk] == b'/' && pk + 1 < scan_lim && src[pk + 1] == b'/' {
                            while pk < scan_lim && src[pk] != b'\n' { pk += 1; }
                            continue;
                        }
                        if src[pk].is_ascii_alphabetic() || src[pk] == b'_' {
                            let r0 = pk;
                            while pk < scan_lim && (src[pk].is_ascii_alphanumeric() || src[pk] == b'_') { pk += 1; }
                            let id = String::from_utf8_lossy(&src[r0..pk]);
                            parts.push(format!("[\"n\",\"{id}\"]"));
                            continue;
                        }
                        pk += 1;
                    }
                    if !parts.is_empty() {
                        let ev = format!("[\"x\",{lno},[\"t\",[0,[\"m\",[{}]]]]]", parts.join(","));
                        if let Some(sym) = self.facts.symbols.get_mut(0) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a27. `if <cond> {` condition calls & identifiers.
                if trimmed.starts_with(b"if ") && !trimmed.starts_with(b"if let ") && trimmed.contains(&b'(') {
                    let rhs = &trimmed[3..];
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < rhs.len() && rhs[rk] != b'{' {
                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                            let r0 = rk;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' { rk += 1; }
                                else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const" | "in" | "let" | "if" | "while" | "match") {
                                if rk < rhs.len() && rhs[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a28. `while <cond> {` condition calls & identifiers.
                if trimmed.starts_with(b"while ") && !trimmed.starts_with(b"while let ") && trimmed.contains(&b'(') {
                    let rhs = &trimmed[6..];
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < rhs.len() && rhs[rk] != b'{' {
                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                            let r0 = rk;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' { rk += 1; }
                                else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const" | "in" | "let" | "if" | "while" | "match") {
                                if rk < rhs.len() && rhs[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a29. Chained `?` try expressions `foo()?.bar(x)?`.
                if !trimmed.starts_with(b"let ") && !trimmed.starts_with(b"return ") && trimmed.contains(&b'?') && trimmed.contains(&b'(') {
                    let rhs = &trimmed[0..];
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < rhs.len() && rhs[rk] != b'{' {
                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                            let r0 = rk;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' { rk += 1; }
                                else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const" | "in" | "let" | "if" | "while" | "match") {
                                if rk < rhs.len() && rhs[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a30. Continuation method chain `.method(args)` lines.
                if trimmed.starts_with(b".") && trimmed.contains(&b'(') {
                    let rhs = &trimmed[1..];
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < rhs.len() && rhs[rk] != b'{' {
                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                            let r0 = rk;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' { rk += 1; }
                                else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const" | "in" | "let" | "if" | "while" | "match") {
                                if rk < rhs.len() && rhs[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a31. `static [mut] <NAME>: <Type> = <expr>;` initializer recovery.
                if (trimmed.starts_with(b"static ") || trimmed.starts_with(b"pub static ")) && trimmed.contains(&b'=') {
                    if let Some(sp) = trimmed.windows(7).position(|w| w == b"static ") {
                        let mut p0 = sp + 7;
                        while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                        if trimmed[p0..].starts_with(b"mut ") { p0 += 4; while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; } }
                        let mut p1 = p0;
                        while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                        if p1 > p0 {
                            let sname = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                            if let Some(eq_rel) = trimmed[p1..].iter().position(|&b| b == b'=') {
                                let rhs = &trimmed[p1 + eq_rel + 1..];
                                let mut parts: Vec<String> = Vec::new();
                                let mut rk = 0usize;
                                while rk < rhs.len() {
                                    if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                        let r0 = rk;
                                        while rk < rhs.len() && (rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' || (rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':')) { rk += 1; }
                                        let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                                        if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "const") {
                                            if rk < rhs.len() && rhs[rk] == b'(' {
                                                let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                    let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                        sym.calls.push(Call { path: full.clone(), line: lno });
                                                        sym.calls.len() - 1
                                                    });
                                                    parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                                }
                                            }
                                            parts.push(format!("[\"n\",\"{full}\"]"));
                                        }
                                        continue;
                                    }
                                    rk += 1;
                                }
                                if !parts.is_empty() {
                                    let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                                    let ev = format!("[\"a\",{lno},[\"{sname}\"],{val},0]");
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        if !sym.events.is_empty() { sym.events.push(','); }
                                        sym.events.push_str(&ev);
                                    }
                                }
                            }
                        }
                    }
                }

                // 3a32. Glob import `use path::*;` recovery.
                if let Some(upos) = trimmed.windows(4).position(|w| w == b"use ") {
                    if (upos == 0 || matches!(trimmed[upos - 1], b' ' | b'\t' | b')')) && trimmed.windows(3).any(|w| w == b"::*") {
                        let rest_u = String::from_utf8_lossy(&trimmed[upos + 4..]);
                        let stmt_u = rest_u.split(';').next().unwrap_or("").trim();
                        if let Some(prefix) = stmt_u.strip_suffix("::*") {
                            let module = prefix.trim().replace("::", ".");
                            if !module.is_empty() && !self.facts.imports.iter().any(|im| im.module == module && im.name == "*") {
                                self.facts.imports.push(Import {
                                    line: lno,
                                    module,
                                    name: "*".to_string(),
                                    alias: String::new(),
                                    kind: IMPORT_MODULE,
                                    ..Import::default()
                                });
                            }
                        }
                    }
                }

                // 3a33. Macro invocations with bracket/brace delimiters `name![...]` / `name!{...}`.
                if let Some(bang) = trimmed.windows(2).position(|w| w == b"![" || w == b"!{") {
                    let mut m0 = bang;
                    while m0 > 0 && (trimmed[m0 - 1].is_ascii_alphanumeric() || trimmed[m0 - 1] == b'_') { m0 -= 1; }
                    if bang > m0 {
                        let mname = String::from_utf8_lossy(&trimmed[m0..bang]).to_string();
                        if !matches!(mname.as_str(), "macro_rules" | "thread_local") {
                            let rest_m = &trimmed[bang + 2..];
                            let mut parts: Vec<String> = vec![format!("[\"n\",\"{mname}\"]")];
                            let mut rk = 0usize;
                            while rk < rest_m.len() && !matches!(rest_m[rk], b']' | b'}') {
                                if rest_m[rk].is_ascii_alphabetic() || rest_m[rk] == b'_' {
                                    let r0 = rk;
                                    while rk < rest_m.len() && (rest_m[rk].is_ascii_alphanumeric() || rest_m[rk] == b'_' || rest_m[rk] == b'.') { rk += 1; }
                                    let id = String::from_utf8_lossy(&rest_m[r0..rk]).trim_matches('.').to_string();
                                    if !id.is_empty() { parts.push(format!("[\"n\",\"{id}\"]")); }
                                    continue;
                                }
                                rk += 1;
                            }
                            if parts.len() >= 2 {
                                let ev = format!("[\"x\",{lno},[\"m\",[{}]]]", parts.join(","));
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            }
                        }
                    }
                }

                // 3a34. Struct update syntax `Type { field, ..base }`.
                if trimmed.contains(&b'{') && trimmed.windows(2).any(|w| w == b"..") && !trimmed.starts_with(b"let ") && trimmed.first().map(|b| b.is_ascii_uppercase()).unwrap_or(false) {
                    let rhs = &trimmed[0..];
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < rhs.len() && rhs[rk] != b'{' {
                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                            let r0 = rk;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' { rk += 1; }
                                else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const" | "in" | "let" | "if" | "while" | "match") {
                                if rk < rhs.len() && rhs[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a35. Multi-line standalone `unsafe {` block inner call & identifier recovery.
                if trimmed == b"unsafe {" || trimmed.starts_with(b"unsafe {") && !trimmed.contains(&b'}') {
                    let abs_b = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + 8;
                    let scan_lim = (abs_b + 500).min(n);
                    let mut pk = abs_b;
                    let mut bd = 1usize;
                    let mut parts: Vec<String> = Vec::new();
                    while pk < scan_lim && bd > 0 {
                        if src[pk] == b'{' { bd += 1; pk += 1; continue; }
                        if src[pk] == b'}' { bd -= 1; pk += 1; continue; }
                        if src[pk].is_ascii_alphabetic() || src[pk] == b'_' {
                            let r0 = pk;
                            while pk < scan_lim && (src[pk].is_ascii_alphanumeric() || src[pk] == b'_' || src[pk] == b'.' || (pk + 1 < scan_lim && src[pk] == b':' && src[pk + 1] == b':')) { pk += 1; }
                            let full = String::from_utf8_lossy(&src[r0..pk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "as" | "ref" | "unsafe" | "const") {
                                if pk < scan_lim && src[pk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        pk += 1;
                    }
                    if !parts.is_empty() {
                        let ev = format!("[\"x\",{lno},[\"m\",[{}]]]", parts.join(","));
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a36. Labeled loops `'label: for|while|loop`.
                if trimmed.starts_with(b"'") && trimmed.windows(2).any(|w| w == b": ") && (trimmed.windows(4).any(|w| w == b"for ") || trimmed.windows(6).any(|w| w == b"while ")) {
                    let rhs = &trimmed[1..];
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < rhs.len() && rhs[rk] != b'{' {
                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                            let r0 = rk;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' { rk += 1; }
                                else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const" | "in" | "let" | "if" | "while" | "match") {
                                if rk < rhs.len() && rhs[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a37. `break <expr>;` value expression recovery.
                if trimmed.starts_with(b"break ") && trimmed.len() > 7 {
                    let rhs = &trimmed[6..];
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < rhs.len() && rhs[rk] != b'{' {
                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                            let r0 = rk;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' { rk += 1; }
                                else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const" | "in" | "let" | "if" | "while" | "match") {
                                if rk < rhs.len() && rhs[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a38. Standalone function call with closure argument `foo(|x| bar(x))`.
                if !trimmed.starts_with(b"let ") && !trimmed.contains(&b'.') && trimmed.contains(&b'(') && trimmed.contains(&b'|') {
                    let rhs = &trimmed[0..];
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < rhs.len() && rhs[rk] != b'{' {
                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                            let r0 = rk;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' { rk += 1; }
                                else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const" | "in" | "let" | "if" | "while" | "match") {
                                if rk < rhs.len() && rhs[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a39. Match arm `Pat => call(...)` expression recovery.
                if trimmed.windows(2).any(|w| w == b"=>") && trimmed.contains(&b'(') && !trimmed.starts_with(b"let ") && !trimmed.starts_with(b"fn ") {
                    let rhs = &trimmed[0..];
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < rhs.len() && rhs[rk] != b'{' {
                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                            let r0 = rk;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.' { rk += 1; }
                                else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false" | "const" | "in" | "let" | "if" | "while" | "match") {
                                if rk < rhs.len() && rhs[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a40. `impl<...> [Trait for] Type` block symbol recovery (host coverage).
                if (trimmed.starts_with(b"impl ") || trimmed.starts_with(b"impl<") || trimmed.starts_with(b"unsafe impl")) && trimmed.contains(&b'{') {
                    let h = String::from_utf8_lossy(trimmed);
                    let before_brace = h.split('{').next().unwrap_or("").trim();
                    let target_part = before_brace.rsplit(" for ").next().unwrap_or(before_brace);
                    let tname = target_part.split('<').next().unwrap_or("").split_whitespace().last().unwrap_or("").trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
                    if !tname.is_empty() && tname.as_bytes()[0].is_ascii_uppercase() {
                        if !self.facts.symbols.iter().any(|sym| sym.name == tname && sym.line == lno) {
                            self.facts.symbols.push(Symbol {
                                key: tname.to_string(),
                                name: tname.to_string(),
                                owner: String::new(),
                                kind: 2,
                                line: lno,
                                end_line: lno,
                                ..Symbol::default()
                            });
                        }
                    }
                }

                // 3a41. Deref-pointer field assignment `(*ptr).field = Call(...)` -> emit `.field` var-type entry.
                if let Some(dp) = trimmed.windows(2).position(|w| w == b").") {
                    if trimmed[..dp].contains(&b'*') && trimmed[dp + 2..].contains(&b'=') {
                        let f0 = dp + 2;
                        let mut f1 = f0;
                        while f1 < trimmed.len() && (trimmed[f1].is_ascii_alphanumeric() || trimmed[f1] == b'_') { f1 += 1; }
                        if f1 > f0 {
                            if let Some(eq_rel) = trimmed[f1..].iter().position(|&b| b == b'=') {
                                let eq = f1 + eq_rel;
                                if eq + 1 < trimmed.len() && trimmed[eq + 1] != b'=' {
                                    let rhs = String::from_utf8_lossy(&trimmed[eq + 1..]).trim().to_string();
                                    let call_name = rhs.split('(').next().unwrap_or("").trim().to_string();
                                    if !call_name.is_empty() && call_name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') && rhs.contains('(') {
                                        let vname = format!(".{}", String::from_utf8_lossy(&trimmed[f0..f1]));
                                        if !self.facts.vars.iter().any(|(vn, vt)| vn == &vname && vt == &call_name) {
                                            self.facts.vars.push((vname, call_name));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // 3a42. `impl Trait for fn(...)` function-pointer trait impl recovery.
                if trimmed.windows(8).any(|w| w == b" for fn(") && trimmed.starts_with(b"impl ") {
                    if !self.facts.symbols.iter().any(|sym| sym.name == "fn" && sym.line == lno) {
                        self.facts.symbols.push(Symbol {
                            key: "fn".to_string(),
                            name: "fn".to_string(),
                            owner: String::new(),
                            kind: 1,
                            line: lno,
                            end_line: lno,
                            ..Symbol::default()
                        });
                    }
                }

                // 3a43. Bare single-segment `use <ident>;` (no `::`, e.g. `pub(crate) use hash_map;`).
                if let Some(upos) = trimmed.windows(4).position(|w| w == b"use ") {
                    if (upos == 0 || matches!(trimmed[upos - 1], b' ' | b'\t' | b')')) && !trimmed[upos + 4..].windows(2).any(|w| w == b"::") {
                        let rest = String::from_utf8_lossy(&trimmed[upos + 4..]).split(';').next().unwrap_or("").trim().to_string();
                        if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                            if !self.facts.imports.iter().any(|im| im.module.is_empty() && im.name.is_empty() && im.alias.is_empty()) {
                                self.facts.imports.push(Import {
                                    line: lno,
                                    module: String::new(),
                                    name: String::new(),
                                    alias: String::new(),
                                    kind: IMPORT_MODULE,
                                    ..Import::default()
                                });
                            }
                        }
                    }
                }

                // 3a45. `.await` async expressions.
                if trimmed.windows(6).any(|w| w == b".await") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a46. `async {` / `async move {` blocks.
                if trimmed.windows(7).any(|w| w == b"async {") || trimmed.windows(12).any(|w| w == b"async move {") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a47. `const {` inline const blocks.
                if trimmed.windows(7).any(|w| w == b"const {") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a48. `Box::new` / `Arc::new` / `Rc::new` allocations.
                if trimmed.windows(8).any(|w| w == b"Box::new" || w == b"Arc::new") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a49. `Ok(`/`Err(`/`Some(` expressions.
                if (trimmed.starts_with(b"Ok(") || trimmed.starts_with(b"Err(") || trimmed.starts_with(b"Some(")) {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a50. Range `..` / `..=` expressions with calls.
                if trimmed.windows(2).any(|w| w == b"..") && trimmed.contains(&b'(') && !trimmed.starts_with(b"let ") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a51. `as` cast expressions with calls.
                if trimmed.windows(4).any(|w| w == b" as ") && trimmed.contains(&b'(') && !trimmed.starts_with(b"let ") && !trimmed.starts_with(b"use ") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a52. `self.` method call statements.
                if trimmed.starts_with(b"self.") && trimmed.contains(&b'(') {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a53. `Self::` associated call statements.
                if trimmed.starts_with(b"Self::") && trimmed.contains(&b'(') {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a54. `crate::`/`super::` qualified call statements.
                if (trimmed.starts_with(b"crate::") || trimmed.starts_with(b"super::")) && trimmed.contains(&b'(') {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a55. `std::`/`core::`/`alloc::` call statements.
                if (trimmed.starts_with(b"std::") || trimmed.starts_with(b"core::") || trimmed.starts_with(b"alloc::")) && trimmed.contains(&b'(') {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a56. `assert!`/`assert_eq!` macro assertions.
                if (trimmed.starts_with(b"assert!(") || trimmed.starts_with(b"assert_eq!(") || trimmed.starts_with(b"assert_ne!(")) {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a57. `panic!`/`unreachable!` diverging macros.
                if (trimmed.starts_with(b"panic!(") || trimmed.starts_with(b"unreachable!(")) {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a58. `write!`/`writeln!`/`format!` formatting macros.
                if (trimmed.starts_with(b"write!(") || trimmed.starts_with(b"writeln!(") || trimmed.starts_with(b"format!(")) {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a59. `matches!(...)` macro expressions.
                if trimmed.windows(9).any(|w| w == b"matches!(") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a60. `vec![...]` macro expressions.
                if trimmed.windows(5).any(|w| w == b"vec![") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a61. `drop(`/`mem::` ownership calls.
                if trimmed.starts_with(b"drop(") || trimmed.starts_with(b"mem::") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a62. `ptr::` raw pointer calls.
                if trimmed.starts_with(b"ptr::") && trimmed.contains(&b'(') {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a63. `let _ = <expr>;` wildcard discard bindings.
                if trimmed.starts_with(b"let _ = ") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3a64. `else if <cond> {` branch conditions.
                if trimmed.starts_with(b"} else if ") || trimmed.starts_with(b"else if ") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() {
                                if trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.' { rk += 1; }
                                else if rk + 1 < trimmed.len() && trimmed[rk] == b':' && trimmed[rk + 1] == b':' { rk += 2; }
                                else { break; }
                            }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).replace("::", ".").trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "let" | "mut" | "ref" | "as" | "unsafe" | "true" | "false" | "const" | "async" | "await" | "move" | "pub" | "crate") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // 3b. `unsafe { ... }` single-line expression -> emit `event x` with all path calls & identifiers.
                if trimmed.starts_with(b"unsafe {") && trimmed.contains(&b'}') {
                    let body = &trimmed[8..];
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < body.len() {
                        if body[rk] == b'}' { break; }
                        if body[rk].is_ascii_alphabetic() || body[rk] == b'_' {
                            let r0 = rk;
                            while rk < body.len() {
                                if body[rk].is_ascii_alphanumeric() || body[rk] == b'_' {
                                    rk += 1;
                                } else if rk + 1 < body.len() && body[rk] == b':' && body[rk + 1] == b':' {
                                    rk += 2;
                                } else {
                                    break;
                                }
                            }
                            let path = String::from_utf8_lossy(&body[r0..rk]).replace("::", ".");
                            let path = path.trim_matches('.').to_string();
                            if !path.is_empty() && !matches!(path.as_str(), "as" | "mut" | "ref" | "const" | "unsafe" | "true" | "false") {
                                if rk < body.len() && body[rk] == b'(' {
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = if let Some(pos) = sym.calls.iter().position(|c| c.line == lno && c.path == path) {
                                            pos
                                        } else {
                                            sym.calls.push(Call { path: path.clone(), line: lno });
                                            sym.calls.len() - 1
                                        };
                                        parts.push(format!("[\"c\",{cidx},0,[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{path}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let ev = format!("[\"x\",{lno},[\"m\",[{}]]]", parts.join(","));
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }
                // 4. `self.<field> = <expr>;` (e.g. `self.filter = Some(::regex::Regex::new(pattern).unwrap());`)
                if trimmed.starts_with(b"self.") && trimmed.contains(&b'=') {
                    if let Some(eq_p) = trimmed.iter().position(|&b| b == b'=') {
                        if eq_p > 5 && eq_p + 1 < trimmed.len() && !matches!(trimmed[eq_p - 1], b'=' | b'!' | b'<' | b'>') && !matches!(trimmed[eq_p + 1], b'=' | b'>') {
                            let target = String::from_utf8_lossy(&trimmed[..eq_p]).trim().to_string();
                            let rhs = &trimmed[eq_p + 1..];
                            let mut parts: Vec<String> = Vec::new();
                            let mut rk = 0usize;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                    let r0 = rk;
                                    while rk < rhs.len() {
                                        if rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' {
                                            rk += 1;
                                        } else if rk + 1 < rhs.len() && rhs[rk] == b':' && rhs[rk + 1] == b':' {
                                            rk += 2;
                                        } else {
                                            break;
                                        }
                                    }
                                    let path = String::from_utf8_lossy(&rhs[r0..rk]).replace("::", ".");
                                    let path = path.trim_matches('.').to_string();
                                    if !path.is_empty() && !matches!(path.as_str(), "as" | "mut" | "ref" | "unsafe" | "true" | "false") {
                                        if rk < rhs.len() && rhs[rk] == b'(' {
                                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                let cidx = if let Some(pos) = sym.calls.iter().position(|c| c.line == lno && c.path == path) {
                                                    pos
                                                } else {
                                                    sym.calls.push(Call { path: path.clone(), line: lno });
                                                    sym.calls.len() - 1
                                                };
                                                parts.push(format!("[\"c\",{cidx},0,[0]]"));
                                            }
                                        }
                                        parts.push(format!("[\"n\",\"{path}\"]"));
                                    }
                                    continue;
                                }
                                rk += 1;
                            }
                            if !parts.is_empty() {
                                let ev = format!("[\"a\",{lno},[\"{target}\"],[\"m\",[{}]],0]", parts.join(","));
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            }
                        }
                    }
                }

            }
            if li < n && src[li] == b'\n' {
                li += 1;
                lno += 1;
            }
        }
    }

    fn augment_java(&mut self) {
        let src = self.orig_src;
        let n = src.len();
        let mut li = 0usize;
        let mut lno = 1u32;
        let mut in_block_comment = false;
        while li < n {
            let line_start = li;
            while li < n && src[li] != b'\n' { li += 1; }
            let raw = &src[line_start..li];
            let line_starts_in_comment = in_block_comment;
            let mut code_end = raw.len();
            let mut k = 0usize;
            let mut in_str = false;
            while k < raw.len() {
                if in_block_comment {
                    if k + 1 < raw.len() && raw[k] == b'*' && raw[k + 1] == b'/' { in_block_comment = false; k += 2; continue; }
                    k += 1;
                    continue;
                }
                if raw[k] == b'"' && (k == 0 || raw[k - 1] != b'\\') { in_str = !in_str; k += 1; continue; }
                if !in_str && k + 1 < raw.len() && raw[k] == b'/' && raw[k + 1] == b'/' { code_end = k; break; }
                if !in_str && k + 1 < raw.len() && raw[k] == b'/' && raw[k + 1] == b'*' { in_block_comment = true; k += 2; continue; }
                k += 1;
            }
            if !line_starts_in_comment {
                let code = &raw[..code_end];
                let trimmed = {
                    let mut t0 = 0;
                    while t0 < code.len() && matches!(code[t0], b' ' | b'\t') { t0 += 1; }
                    &code[t0..]
                };
                let owner_idx = self
                    .facts
                    .symbols
                    .iter()
                    .enumerate()
                    .skip(1)
                    .filter(|(_, sym)| sym.line <= lno && lno <= sym.end_line)
                    .max_by_key(|(_, sym)| sym.line)
                    .map(|(idx, _)| idx)
                    .unwrap_or(0);

                // J1. Java shift-assignment statements (`>>=`, `>>>=`, `<<=`) e.g. `a >>= aTwos;`, `value >>= 8;`.
                if let Some((op_pos, op_len)) = trimmed.windows(4).position(|w| w == b">>>=").map(|p| (p, 4))
                    .or_else(|| trimmed.windows(3).position(|w| w == b">>=" || w == b"<<=").map(|p| (p, 3)))
                {
                    let lhs = String::from_utf8_lossy(&trimmed[..op_pos]).trim().to_string();
                    if !lhs.is_empty() && (lhs.as_bytes()[0].is_ascii_alphabetic() || lhs.as_bytes()[0] == b'_')
                        && lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
                    {
                        let rhs = &trimmed[op_pos + op_len..];
                        let mut parts: Vec<String> = Vec::new();
                        let mut rk = 0usize;
                        while rk < rhs.len() && rhs[rk] != b';' {
                            if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                let r0 = rk;
                                while rk < rhs.len() && (rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.') { rk += 1; }
                                let id = String::from_utf8_lossy(&rhs[r0..rk]).trim_matches('.').to_string();
                                if !id.is_empty() { parts.push(format!("[\"n\",\"{id}\"]")); }
                                continue;
                            }
                            rk += 1;
                        }
                        let val = if parts.is_empty() { "0".to_string() } else if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"a\",{lno},[\"{lhs}\"],{val},0]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J2. Java `return <expr>;` statements (including casts `(int) ...` and generic type-witness calls `.<T>method(...)`).
                if trimmed.starts_with(b"return ") && trimmed.len() > 8 {
                    let rhs = &trimmed[7..];
                    let mut first_call: Option<String> = None;
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < rhs.len() && rhs[rk] != b';' {
                        if rhs[rk] == b'"' {
                            rk += 1;
                            while rk < rhs.len() && rhs[rk] != b'"' { if rhs[rk] == b'\\' && rk + 1 < rhs.len() { rk += 2; continue; } rk += 1; }
                            if rk < rhs.len() { rk += 1; }
                            parts.push("0".to_string());
                            continue;
                        }
                        if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                            let r0 = rk;
                            while rk < rhs.len() && (rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&rhs[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "new" | "true" | "false" | "null" | "instanceof") {
                                if rk < rhs.len() && rhs[rk] == b'(' {
                                    if first_call.is_none() {
                                        first_call = Some(full.clone());
                                    } else {
                                        let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                            let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                sym.calls.push(Call { path: full.clone(), line: lno });
                                                sym.calls.len() - 1
                                            });
                                            parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                        }
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if let Some(fc) = first_call {
                        let recv = fc.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == fc).unwrap_or_else(|| {
                                sym.calls.push(Call { path: fc.clone(), line: lno });
                                sym.calls.len() - 1
                            });
                            let aexpr = if parts.is_empty() { "0".to_string() } else { format!("[\"m\",[{}]]", parts.join(",")) };
                            let ev = format!("[\"r\",{lno},[\"c\",{cidx},{recv},[{aexpr}]]]");
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                            let ev2 = format!("[\"r\",{lno},{aexpr}]");
                            sym.events.push(',');
                            sym.events.push_str(&ev2);
                        }
                    } else if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"r\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J3. Java reassignment with call args `lhs = call(arg1, arg2, ...);` (e.g. `hour = parseInt(date, offset += 1, offset += 2);`).
                if !trimmed.starts_with(b"if ") && !trimmed.starts_with(b"while ") && !trimmed.starts_with(b"for ")
                    && !trimmed.starts_with(b"return ") && trimmed.contains(&b'=') && trimmed.contains(&b'(')
                {
                    if let Some(eq_p) = trimmed.iter().position(|&b| b == b'=') {
                        let prev_ok = eq_p > 0 && !matches!(trimmed[eq_p - 1], b'=' | b'!' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/' | b'&' | b'|' | b'^');
                        let next_ok = eq_p + 1 < trimmed.len() && trimmed[eq_p + 1] != b'=';
                        if prev_ok && next_ok {
                            let lhs_raw = String::from_utf8_lossy(&trimmed[..eq_p]).trim().to_string();
                            let lhs = lhs_raw.split_whitespace().last().unwrap_or("").to_string();
                            if !lhs.is_empty() && (lhs.as_bytes()[0].is_ascii_alphabetic() || lhs.as_bytes()[0] == b'_')
                                && lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
                            {
                                let rhs = &trimmed[eq_p + 1..];
                                let mut first_call: Option<String> = None;
                                let mut parts: Vec<String> = Vec::new();
                                let mut rk = 0usize;
                                while rk < rhs.len() && rhs[rk] != b';' {
                                    if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                        let r0 = rk;
                                        while rk < rhs.len() && (rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.') { rk += 1; }
                                        let full = String::from_utf8_lossy(&rhs[r0..rk]).trim_matches('.').to_string();
                                        if !full.is_empty() && !matches!(full.as_str(), "new" | "true" | "false" | "null") {
                                            if first_call.is_none() && rk < rhs.len() && rhs[rk] == b'(' {
                                                first_call = Some(full);
                                            } else {
                                                parts.push(format!("[\"n\",\"{full}\"]"));
                                            }
                                        }
                                        continue;
                                    }
                                    rk += 1;
                                }
                                if let Some(fc) = first_call {
                                    let recv = fc.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == fc).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: fc.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        let aexpr = if parts.is_empty() { "0".to_string() } else { format!("[\"m\",[{}]]", parts.join(",")) };
                                        let ev = format!("[\"a\",{lno},[\"{lhs}\"],[\"c\",{cidx},{recv},[{aexpr}]],0]");
                                        if !sym.events.is_empty() { sym.events.push(','); }
                                        sym.events.push_str(&ev);
                                    }
                                }
                            }
                        }
                    }
                }

                // J4. Java `import [static] a.b.Item;` & wildcard `import a.b.*;`.
                if trimmed.starts_with(b"import ") {
                    let rest = String::from_utf8_lossy(&trimmed[7..]).trim().trim_start_matches("static ").trim().trim_end_matches(';').to_string();
                    if let Some(mod_p) = rest.strip_suffix(".*") {
                        let module = mod_p.trim().to_string();
                        if !module.is_empty() && !self.facts.imports.iter().any(|im| im.module == module && im.name.is_empty()) {
                            self.facts.imports.push(Import { line: lno, module, name: String::new(), alias: String::new(), kind: IMPORT_MODULE, ..Import::default() });
                        }
                    } else if let Some((mod_p, item_p)) = rest.rsplit_once('.') {
                        if !item_p.is_empty() && !self.facts.imports.iter().any(|im| im.module == mod_p && im.name == item_p) {
                            self.facts.imports.push(Import { line: lno, module: mod_p.to_string(), name: item_p.to_string(), alias: item_p.to_string(), kind: IMPORT_MODULE, ..Import::default() });
                        }
                    }
                }

                // J12. Java `try (` try-with-resources expressions.
                if trimmed.starts_with(b"try (") || trimmed.starts_with(b"try(") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J5. Java `class`/`interface`/`enum`/`record` symbol recovery.
                for &(kw, skind) in &[(b"class " as &[u8], 2u8), (b"interface ", 2u8), (b"enum ", 2u8), (b"record ", 2u8)] {
                    if let Some(kpos) = trimmed.windows(kw.len()).position(|w| w == kw) {
                        if kpos == 0 || matches!(trimmed[kpos - 1], b' ' | b'\t') {
                            let mut p0 = kpos + kw.len();
                            while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                            let mut p1 = p0;
                            while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                            if p1 > p0 && trimmed[p0].is_ascii_alphabetic() {
                                let sname = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                                if !self.facts.symbols.iter().any(|sym| sym.name == sname && sym.line == lno) {
                                    let owner = self.facts.symbols.get(owner_idx).map(|s| s.key.clone()).unwrap_or_default();
                                    let key = if owner.is_empty() { sname.clone() } else { format!("{owner}.{sname}") };
                                    self.facts.symbols.push(Symbol { key, name: sname, owner, kind: skind, line: lno, end_line: lno, ..Symbol::default() });
                                }
                            }
                        }
                    }
                }

                // J6. Java `new Type(...)` variable type recovery (`self.facts.vars`).
                if trimmed.windows(5).any(|w| w == b" new ") && trimmed.contains(&b'=') {
                    if let (Some(eq_p), Some(npos)) = (trimmed.iter().position(|&b| b == b'='), trimmed.windows(5).position(|w| w == b" new ")) {
                        if npos > eq_p {
                            let lhs_raw = String::from_utf8_lossy(&trimmed[..eq_p]).trim().to_string();
                            let vname = lhs_raw.split_whitespace().last().unwrap_or("").to_string();
                            let rest = String::from_utf8_lossy(&trimmed[npos + 5..]);
                            let tname = rest.split(|c: char| c == '(' || c == '<' || c == '[' || c == ' ').next().unwrap_or("").trim().to_string();
                            if !vname.is_empty() && !tname.is_empty() && tname.as_bytes()[0].is_ascii_uppercase() {
                                if !self.facts.vars.iter().any(|(n, t)| n == &vname && t == &tname) {
                                    self.facts.vars.push((vname, tname));
                                }
                            }
                        }
                    }
                }

                // J7. Java compound assignments (`+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`).
                if trimmed.windows(2).any(|w| matches!(w, b"+=" | b"-=" | b"*=" | b"/=" | b"%=" | b"&=" | b"|=" | b"^=")) {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J8. Java `if (` condition calls & identifiers.
                if trimmed.starts_with(b"if ") || trimmed.starts_with(b"if(") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J9. Java `while (` loop conditions.
                if trimmed.starts_with(b"while ") || trimmed.starts_with(b"while(") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J10. Java `for (` / enhanced-for loop headers.
                if trimmed.starts_with(b"for ") || trimmed.starts_with(b"for(") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J11. Java `switch (` scrutinee expressions.
                if trimmed.starts_with(b"switch ") || trimmed.starts_with(b"switch(") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J12. Java `try (` try-with-resources expressions.
                if trimmed.starts_with(b"try (") || trimmed.starts_with(b"try(") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J13. Java `throw new ...` exception statements.
                if trimmed.starts_with(b"throw ") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J14. Java `assert` statements.
                if trimmed.starts_with(b"assert ") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J15. Java `synchronized (` monitor expressions.
                if trimmed.starts_with(b"synchronized ") || trimmed.starts_with(b"synchronized(") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J16. Java lambda `->` expressions.
                if trimmed.windows(2).any(|w| w == b"->") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J17. Java method references `Class::method`.
                if trimmed.windows(2).any(|w| w == b"::") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J18. Java continuation method chains `.method(...)`.
                if trimmed.starts_with(b".") && trimmed.contains(&b'(') {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J19. Java ternary `? :` expressions.
                if trimmed.contains(&b'?') && trimmed.contains(&b':') && !trimmed.starts_with(b"case ") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J20. Java `super(...)` / `this(...)` constructor delegation.
                if trimmed.starts_with(b"super(") || trimmed.starts_with(b"this(") || trimmed.starts_with(b"super.") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J21. Java `case` switch labels & switch-expression arms.
                if trimmed.starts_with(b"case ") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J22. Java `instanceof` pattern-matching expressions.
                if trimmed.windows(12).any(|w| w == b" instanceof ") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J23. Java `yield` switch-expression statements.
                if trimmed.starts_with(b"yield ") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J24. Java `new Type[]` / anonymous class `new Type() {` expressions.
                if trimmed.windows(5).any(|w| w == b" new ") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // J25. Java `catch (` multi-catch exception handlers.
                if trimmed.starts_with(b"catch ") || trimmed.starts_with(b"} catch") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() && trimmed[rk] != b';' {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "switch" | "case" | "try" | "catch" | "new" | "throw" | "return" | "true" | "false" | "null" | "synchronized" | "assert" | "instanceof") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

            // JAVA_NEXT_RULE
            }
            if li < n && src[li] == b'\n' { li += 1; lno += 1; }
        }
    }

    fn augment_kotlin(&mut self) {
        let src = self.orig_src;
        let n = src.len();
        let mut li = 0usize;
        let mut lno = 1u32;
        let mut in_block_comment = 0usize;
        while li < n {
            let line_start = li;
            while li < n && src[li] != b'\n' { li += 1; }
            let raw = &src[line_start..li];
            let line_starts_in_comment = in_block_comment > 0;
            let mut code_end = raw.len();
            let mut k = 0usize;
            let mut in_str = false;
            while k < raw.len() {
                if in_block_comment > 0 {
                    if k + 1 < raw.len() && raw[k] == b'/' && raw[k + 1] == b'*' { in_block_comment += 1; k += 2; continue; }
                    if k + 1 < raw.len() && raw[k] == b'*' && raw[k + 1] == b'/' { in_block_comment -= 1; k += 2; continue; }
                    k += 1;
                    continue;
                }
                if raw[k] == b'"' && (k == 0 || raw[k - 1] != b'\\') { in_str = !in_str; k += 1; continue; }
                if !in_str && k + 1 < raw.len() && raw[k] == b'/' && raw[k + 1] == b'/' { code_end = k; break; }
                if !in_str && k + 1 < raw.len() && raw[k] == b'/' && raw[k + 1] == b'*' { in_block_comment += 1; k += 2; continue; }
                k += 1;
            }
            if !line_starts_in_comment {
                let code = &raw[..code_end];
                let trimmed = {
                    let mut t0 = 0;
                    while t0 < code.len() && matches!(code[t0], b' ' | b'\t') { t0 += 1; }
                    &code[t0..]
                };
                let owner_idx = self
                    .facts
                    .symbols
                    .iter()
                    .enumerate()
                    .skip(1)
                    .filter(|(_, sym)| sym.line <= lno && lno <= sym.end_line)
                    .max_by_key(|(_, sym)| sym.line)
                    .map(|(idx, _)| idx)
                    .unwrap_or(0);

                // K1. Kotlin expression-line call scanning (dotted, unqualified, generic `<T>(...)`, inside lambdas/branches).
                if trimmed.contains(&b'(') && !trimmed.starts_with(b"fun ") && !trimmed.starts_with(b"class ")
                    && !trimmed.starts_with(b"interface ") && !trimmed.starts_with(b"object ")
                {
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            let mut lk = rk;
                            if lk < trimmed.len() && trimmed[lk] == b'<' {
                                let mut ad = 1usize; lk += 1;
                                while lk < trimmed.len() && ad > 0 {
                                    if trimmed[lk] == b'<' { ad += 1; } else if trimmed[lk] == b'>' { ad -= 1; }
                                    lk += 1;
                                }
                            }
                            if !full.is_empty() && lk < trimmed.len() && trimmed[lk] == b'('
                                && !matches!(full.as_str(), "if" | "while" | "for" | "when" | "catch" | "fun" | "class" | "constructor" | "synchronized")
                            {
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.calls.iter().any(|c| c.line == lno && c.path == full) {
                                        sym.calls.push(Call { path: full, line: lno });
                                    }
                                }
                            }
                            continue;
                        }
                        rk += 1;
                    }
                }

                // K2. Kotlin property getter/setter `get() = expr` / `set(v) = expr` (emitted to symbol 0 + owner).
                if trimmed.starts_with(b"get()") || trimmed.starts_with(b"set(") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "when" | "while" | "for" | "in" | "is" | "as" | "true" | "false" | "null" | "return" | "throw" | "try" | "catch") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(0) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(0) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // K3. Kotlin `val`/`var` bindings (single/multi-line `when`/`if`/block RHS, emitted to owner + symbol 0 when class-level).
                if let Some(vpos) = trimmed.windows(4).position(|w| w == b"val " || w == b"var ") {
                    if (vpos == 0 || matches!(trimmed[vpos - 1], b' ' | b'\t')) && trimmed[vpos + 4..].contains(&b'=') {
                        let mut p0 = vpos + 4;
                        while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                        let mut p1 = p0;
                        while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                        if p1 > p0 && (trimmed[p0].is_ascii_alphabetic() || trimmed[p0] == b'_') {
                            let target = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                            if let Some(eq_rel) = trimmed[p1..].iter().position(|&b| b == b'=') {
                                let eq = p1 + eq_rel;
                                if eq + 1 < trimmed.len() && !matches!(trimmed[eq + 1], b'=' | b'>') {
                                    let abs_eq = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + eq;
                                    let scan_lim = (abs_eq + 900).min(n);
                                    let mut pk = abs_eq + 1;
                                    let mut bd = 0usize;
                                    let mut seen_b = false;
                                    let mut parts: Vec<String> = Vec::new();
                                    while pk < scan_lim {
                                        if src[pk] == b'\n' && bd == 0 && !seen_b && !parts.is_empty() { break; }
                                        if src[pk] == b'{' { bd += 1; seen_b = true; pk += 1; continue; }
                                        if src[pk] == b'}' { if bd > 0 { bd -= 1; } pk += 1; if seen_b && bd == 0 { break; } continue; }
                                        if src[pk].is_ascii_alphabetic() || src[pk] == b'_' {
                                            let r0 = pk;
                                            while pk < scan_lim && (src[pk].is_ascii_alphanumeric() || src[pk] == b'_' || src[pk] == b'.') { pk += 1; }
                                            let full = String::from_utf8_lossy(&src[r0..pk]).trim_matches('.').to_string();
                                            if !full.is_empty() && !matches!(full.as_str(), "as" | "is" | "in" | "true" | "false" | "null") {
                                                if pk < scan_lim && src[pk] == b'(' {
                                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                                            sym.calls.len() - 1
                                                        });
                                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                                    }
                                                }
                                                parts.push(format!("[\"n\",\"{full}\"]"));
                                            }
                                            continue;
                                        }
                                        pk += 1;
                                    }
                                    if !parts.is_empty() {
                                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                                        let ev = format!("[\"a\",{lno},[\"{target}\"],{val},0]");
                                        for si in [owner_idx, 0usize] {
                                            if si == 0 && owner_idx == 0 { continue; }
                                            if let Some(sym) = self.facts.symbols.get_mut(si) {
                                                if !sym.events.is_empty() { sym.events.push(','); }
                                                sym.events.push_str(&ev);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // K4. Kotlin trailing-lambda/DSL block expressions (`buildPacket {`, `onMessage {`, `when (x) {`).
                if trimmed.contains(&b'{') && !trimmed.starts_with(b"class ") && !trimmed.starts_with(b"fun ") && !trimmed.starts_with(b"interface ") && !trimmed.starts_with(b"object ") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "when" | "while" | "for" | "in" | "is" | "as" | "true" | "false" | "null" | "return" | "throw" | "try" | "catch") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(0) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(0) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // K5. Kotlin `->` lambda / `when`-branch expressions (emitted to symbol 0).
                if trimmed.windows(2).any(|w| w == b"->") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "when" | "while" | "for" | "in" | "is" | "as" | "true" | "false" | "null" | "return" | "throw" | "try" | "catch") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(0) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(0) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // K6. Kotlin `->` lambda / `when`-branch expressions (emitted to owner_idx).
                if trimmed.windows(2).any(|w| w == b"->") {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    let mut instr = false;
                    while rk < trimmed.len() {
                        if trimmed[rk] == b'"' && (rk == 0 || trimmed[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                        if instr { rk += 1; continue; }
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let full = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "when" | "while" | "for" | "in" | "is" | "as" | "true" | "false" | "null" | "return" | "throw" | "try" | "catch") {
                                if rk < trimmed.len() && trimmed[rk] == b'(' {
                                    let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                            sym.calls.push(Call { path: full.clone(), line: lno });
                                            sym.calls.len() - 1
                                        });
                                        parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                                    }
                                }
                                parts.push(format!("[\"n\",\"{full}\"]"));
                            }
                            continue;
                        }
                        rk += 1;
                    }
                    if !parts.is_empty() {
                        let val = if parts.len() == 1 { parts.remove(0) } else { format!("[\"m\",[{}]]", parts.join(",")) };
                        let ev = format!("[\"x\",{lno},{val}]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
                        }
                    }
                }

                // K7. Kotlin `fun <name>` symbol recovery (including `suspend`/`inline`/`override`/`private` & extension `Type.name`).
                if let Some(fpos) = trimmed.windows(4).position(|w| w == b"fun ") {
                    if fpos == 0 || matches!(trimmed[fpos - 1], b' ' | b'\t') {
                        let rest = String::from_utf8_lossy(&trimmed[fpos + 4..]);
                        let before_lp = rest.split('(').next().unwrap_or("").trim();
                        let fname = before_lp.rsplit('.').next().unwrap_or("").split('<').next().unwrap_or("").trim().to_string();
                        if !fname.is_empty() && (fname.as_bytes()[0].is_ascii_alphabetic() || fname.as_bytes()[0] == b'_')
                            && fname.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                        {
                            if !self.facts.symbols.iter().any(|sym| sym.name == fname && sym.line == lno) {
                                let mut eline = lno;
                                let mut bk = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + fpos + 4;
                                let lim = (bk + 1500).min(n);
                                while bk < lim && !matches!(src[bk], b'{' | b'\n') { bk += 1; }
                                if bk < lim && src[bk] == b'{' {
                                    let mut d = 1usize; bk += 1;
                                    while bk < n && d > 0 { match src[bk] { b'\n' => eline += 1, b'{' => d += 1, b'}' => d -= 1, _ => {} } bk += 1; }
                                }
                                let owner = self.facts.symbols.get(owner_idx).map(|s| s.key.clone()).unwrap_or_default();
                                let key = if owner.is_empty() { fname.clone() } else { format!("{owner}.{fname}") };
                                self.facts.symbols.push(Symbol { key, name: fname, owner, kind: 1, line: lno, end_line: eline, ..Symbol::default() });
                            }
                        }
                    }
                }

                // K8. Kotlin `class`/`interface`/`object`/`typealias` symbol recovery.
                for &(kw, skind) in &[(b"class " as &[u8], 2u8), (b"interface ", 2u8), (b"object ", 2u8), (b"typealias ", 2u8)] {
                    if let Some(kpos) = trimmed.windows(kw.len()).position(|w| w == kw) {
                        if kpos == 0 || matches!(trimmed[kpos - 1], b' ' | b'\t') {
                            let mut p0 = kpos + kw.len();
