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

    fn augment_scala(&mut self) {
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

                // S1. Scala wildcard imports `import a.b.*` and `import a.b._`.
                if trimmed.starts_with(b"import ") {
                    let rest = String::from_utf8_lossy(&trimmed[7..]).trim().trim_end_matches(';').to_string();
                    if let Some(mod_p) = rest.strip_suffix(".*").or_else(|| rest.strip_suffix("._")) {
                        let module = mod_p.trim().to_string();
                        if !module.is_empty() && !self.facts.imports.iter().any(|im| im.module == module && im.name.is_empty() && im.alias.is_empty()) {
                            self.facts.imports.push(Import {
                                line: lno,
                                module,
                                name: String::new(),
                                alias: String::new(),
                                kind: IMPORT_MODULE,
                                ..Import::default()
                            });
                        }
                    }
                }

                // S2. Scala brace & renamed imports `import a.b.{c, d => e, f as g, _}`.
                if trimmed.starts_with(b"import ") && trimmed.contains(&b'{') {
                    let rest = String::from_utf8_lossy(&trimmed[7..]);
                    if let (Some(lb), Some(rb)) = (rest.find('{'), rest.rfind('}')) {
                        if rb > lb {
                            let prefix = rest[..lb].trim().trim_end_matches('.').to_string();
                            for item in rest[lb + 1..rb].split(',') {
                                let it = item.trim();
                                if it.is_empty() { continue; }
                                if it == "_" || it == "*" {
                                    if !self.facts.imports.iter().any(|im| im.module == prefix && im.name.is_empty()) {
                                        self.facts.imports.push(Import { line: lno, module: prefix.clone(), name: String::new(), alias: String::new(), kind: IMPORT_MODULE, ..Import::default() });
                                    }
                                } else {
                                    let (iname, ialias) = if let Some((a, b)) = it.split_once("=>").or_else(|| it.split_once(" as ")) {
                                        (a.trim().to_string(), b.trim().to_string())
                                    } else {
                                        (it.to_string(), it.to_string())
                                    };
                                    if !iname.is_empty() && ialias != "_" && iname.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                                        if !self.facts.imports.iter().any(|im| im.module == prefix && im.name == iname) {
                                            self.facts.imports.push(Import { line: lno, module: prefix.clone(), name: iname, alias: ialias, kind: IMPORT_MODULE, ..Import::default() });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // S3. Scala string interpolation `${...}` calls (e.g. `log"${MDC(k, v)}"`, `s"${foo(x)}"`).
                if trimmed.windows(2).any(|w| w == b"${") {
                    let mut sk = 0usize;
                    while sk + 2 < trimmed.len() {
                        if trimmed[sk] == b'$' && trimmed[sk + 1] == b'{' {
                            sk += 2;
                            let mut bd = 1usize;
                            while sk < trimmed.len() && bd > 0 {
                                if trimmed[sk] == b'{' { bd += 1; sk += 1; continue; }
                                if trimmed[sk] == b'}' { bd -= 1; sk += 1; continue; }
                                if trimmed[sk].is_ascii_alphabetic() || trimmed[sk] == b'_' {
                                    let r0 = sk;
                                    while sk < trimmed.len() && (trimmed[sk].is_ascii_alphanumeric() || trimmed[sk] == b'_' || trimmed[sk] == b'.') { sk += 1; }
                                    let full = String::from_utf8_lossy(&trimmed[r0..sk]).trim_matches('.').to_string();
                                    if !full.is_empty() && sk < trimmed.len() && trimmed[sk] == b'(' {
                                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                            if !sym.calls.iter().any(|c| c.line == lno && c.path == full) {
                                                sym.calls.push(Call { path: full, line: lno });
                                            }
                                        }
                                    }
                                    continue;
                                }
                                sk += 1;
                            }
                            continue;
                        }
                        sk += 1;
                    }
                }

                // S4. Scala `val` / `var` bindings (including `private`/`lazy`/`override`/`implicit` & multi-line block `{ ... }` RHS).
                if let Some(vpos) = trimmed.windows(4).position(|w| w == b"val " || w == b"var ") {
                    let prev_ok = vpos == 0 || matches!(trimmed[vpos - 1], b' ' | b'\t');
                    if prev_ok && trimmed[vpos + 4..].contains(&b'=') {
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
                                        if src[pk] == b'}' {
                                            if bd > 0 { bd -= 1; }
                                            pk += 1;
                                            if seen_b && bd == 0 { break; }
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
                                            while pk < scan_lim && (src[pk].is_ascii_alphanumeric() || src[pk] == b'_' || src[pk] == b'.') { pk += 1; }
                                            let full = String::from_utf8_lossy(&src[r0..pk]).trim_matches('.').to_string();
                                            if !full.is_empty() && !matches!(full.as_str(), "new" | "val" | "var" | "def" | "true" | "false" | "null" | "match" | "case" | "if" | "else") {
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

                // S5. Scala tuple destructuring `val (a, b, ...) = expr` (single/multi-line RHS).
                if let Some(vpos) = trimmed.windows(5).position(|w| w == b"val (" || w == b"var (") {
                    if let (Some(rp_rel), Some(eq_rel)) = (trimmed[vpos + 5..].iter().position(|&b| b == b')'), trimmed[vpos + 5..].iter().position(|&b| b == b'=')) {
                        if eq_rel > rp_rel {
                            let pat_s = String::from_utf8_lossy(&trimmed[vpos + 5..vpos + 5 + rp_rel]);
                            let mut tgts: Vec<String> = Vec::new();
                            for part in pat_s.split(',') {
                                let p = part.trim().split(':').next().unwrap_or("").trim();
                                if !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                                    tgts.push(format!("\"{p}\""));
                                }
                            }
                            if !tgts.is_empty() {
                                let abs_eq = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + vpos + 5 + eq_rel;
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
                                        if !full.is_empty() && !matches!(full.as_str(), "new" | "val" | "var" | "true" | "false" | "null") {
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

                // S6. Scala `def <name>` symbol recovery (with brace `end_line` tracking).
                if let Some(dpos) = trimmed.windows(4).position(|w| w == b"def ") {
                    let prev_ok = dpos == 0 || matches!(trimmed[dpos - 1], b' ' | b'\t');
                    if prev_ok {
                        let mut p0 = dpos + 4;
                        while p0 < trimmed.len() && matches!(trimmed[p0], b' ' | b'\t') { p0 += 1; }
                        let mut p1 = p0;
                        while p1 < trimmed.len() && (trimmed[p1].is_ascii_alphanumeric() || trimmed[p1] == b'_') { p1 += 1; }
                        if p1 > p0 && (trimmed[p0].is_ascii_alphabetic() || trimmed[p0] == b'_') {
                            let dname = String::from_utf8_lossy(&trimmed[p0..p1]).to_string();
                            if !self.facts.symbols.iter().any(|sym| sym.name == dname && sym.line == lno) {
                                let mut eline = lno;
                                let mut bk = line_start + (trimmed.as_ptr() as usize - raw.as_ptr() as usize) + p1;
                                let lim = (bk + 2000).min(n);
                                while bk < lim && !matches!(src[bk], b'{' | b'\n') { bk += 1; }
                                if bk < lim && src[bk] == b'{' {
                                    let mut depth = 1usize; bk += 1;
                                    while bk < n && depth > 0 {
                                        match src[bk] { b'\n' => eline += 1, b'{' => depth += 1, b'}' => depth -= 1, _ => {} }
                                        bk += 1;
                                    }
                                }
                                let owner = self.facts.symbols.get(owner_idx).map(|s| s.key.clone()).unwrap_or_default();
                                let key = if owner.is_empty() { dname.clone() } else { format!("{owner}.{dname}") };
                                self.facts.symbols.push(Symbol { key, name: dname, owner, kind: 1, line: lno, end_line: eline, ..Symbol::default() });
                            }
                        }
                    }
                }

                // S7. Scala `class`/`trait`/`object`/`enum`/`type`/`given` symbol recovery.
                for &(kw, skind) in &[
                    (b"class " as &[u8], 2u8), (b"trait ", 2u8), (b"object ", 2u8),
                    (b"enum ", 2u8), (b"type ", 1u8), (b"given ", 1u8),
                ] {
                    if let Some(kpos) = trimmed.windows(kw.len()).position(|w| w == kw) {
                        let prev_ok = kpos == 0 || matches!(trimmed[kpos - 1], b' ' | b'\t');
                        if prev_ok {
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

                // S8. Scala call scanning on lines with `(` (dotted & unqualified calls, including inside infix `->`, `+=`, `JArray(x.map(...))`).
                if trimmed.contains(&b'(') && !trimmed.starts_with(b"def ") && !trimmed.starts_with(b"class ") && !trimmed.starts_with(b"case class ") {
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
                            if lk < trimmed.len() && trimmed[lk] == b'[' {
                                let mut bd = 1usize; lk += 1;
                                while lk < trimmed.len() && bd > 0 {
                                    if trimmed[lk] == b'[' { bd += 1; } else if trimmed[lk] == b']' { bd -= 1; }
                                    lk += 1;
                                }
                            }
                            if !full.is_empty() && lk < trimmed.len() && trimmed[lk] == b'('
                                && !matches!(full.as_str(), "if" | "while" | "for" | "catch" | "match" | "case" | "def" | "class" | "new")
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

                // S10. Scala `private`/`protected`/`override`/`final` `val` expression fallback `event x` (when lexer sees modifier tokens).
                if (trimmed.starts_with(b"private ") || trimmed.starts_with(b"protected ") || trimmed.starts_with(b"override ") || trimmed.starts_with(b"final "))
                    && (trimmed.windows(4).any(|w| w == b"val ") || trimmed.windows(4).any(|w| w == b"var "))
                {
                    let mut parts: Vec<String> = Vec::new();
                    let mut rk = 0usize;
                    while rk < trimmed.len() {
                        if trimmed[rk].is_ascii_alphabetic() || trimmed[rk] == b'_' {
                            let r0 = rk;
                            while rk < trimmed.len() && (trimmed[rk].is_ascii_alphanumeric() || trimmed[rk] == b'_' || trimmed[rk] == b'.') { rk += 1; }
                            let id = String::from_utf8_lossy(&trimmed[r0..rk]).trim_matches('.').to_string();
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

                // S11. Scala `match {` expression scrutinee recovery.
                if trimmed.windows(6).any(|w| w == b" match") {
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
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "match" | "case" | "try" | "catch" | "new" | "true" | "false" | "null" | "return" | "throw") {
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

                // S12. Scala `for (` / `for {` comprehension generator recovery.
                if trimmed.starts_with(b"for ") || trimmed.starts_with(b"for(") {
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
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "match" | "case" | "try" | "catch" | "new" | "true" | "false" | "null" | "return" | "throw") {
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

                // S13. Scala `case ... =>` pattern arm expression recovery.
                if trimmed.starts_with(b"case ") && trimmed.windows(2).any(|w| w == b"=>") {
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
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "match" | "case" | "try" | "catch" | "new" | "true" | "false" | "null" | "return" | "throw") {
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

                // S14. Scala `=>` lambda/closure expression recovery.
                if !trimmed.starts_with(b"case ") && trimmed.windows(2).any(|w| w == b"=>") {
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
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "match" | "case" | "try" | "catch" | "new" | "true" | "false" | "null" | "return" | "throw") {
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

                // S15. Scala `<-` for-comprehension generator binding recovery.
                if trimmed.windows(2).any(|w| w == b"<-") {
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
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "match" | "case" | "try" | "catch" | "new" | "true" | "false" | "null" | "return" | "throw") {
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

                // S16. Scala `+=` / `-=` / `++=` collection mutation statements.
                if trimmed.windows(2).any(|w| w == b"+=" || w == b"-=") {
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
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "match" | "case" | "try" | "catch" | "new" | "true" | "false" | "null" | "return" | "throw") {
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

                // S17. Scala `return expr` statements.
                if trimmed.starts_with(b"return ") {
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
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "match" | "case" | "try" | "catch" | "new" | "true" | "false" | "null" | "return" | "throw") {
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

                // S18. Scala `throw new ...` exception statements.
                if trimmed.starts_with(b"throw ") {
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
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "match" | "case" | "try" | "catch" | "new" | "true" | "false" | "null" | "return" | "throw") {
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

                // S19. Scala `if (` / `if ... then` conditional expressions.
                if trimmed.starts_with(b"if ") || trimmed.starts_with(b"if(") {
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
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "match" | "case" | "try" | "catch" | "new" | "true" | "false" | "null" | "return" | "throw") {
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

                // S20. Scala `while (` / `while ... do` loop conditions.
                if trimmed.starts_with(b"while ") || trimmed.starts_with(b"while(") {
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
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "match" | "case" | "try" | "catch" | "new" | "true" | "false" | "null" | "return" | "throw") {
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

                // S21. Scala continuation method chain `.method(...)` lines.
                if trimmed.starts_with(b".") && trimmed.contains(&b'(') {
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
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "match" | "case" | "try" | "catch" | "new" | "true" | "false" | "null" | "return" | "throw") {
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

                // S22. Scala single-item imports `import a.b.Item` / `import a.b.Item as Alias`.
                if trimmed.starts_with(b"import ") && !trimmed.contains(&b'{') && !trimmed.ends_with(b".*") && !trimmed.ends_with(b"._") {
                    let rest = String::from_utf8_lossy(&trimmed[7..]).trim().trim_end_matches(';').to_string();
                    let (path_s, alias_s) = if let Some((a, b)) = rest.split_once(" as ") {
                        (a.trim().to_string(), b.trim().to_string())
                    } else {
                        (rest.clone(), String::new())
                    };
                    if let Some((mod_p, item_p)) = path_s.rsplit_once('.') {
                        if !item_p.is_empty() && item_p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                            let al = if alias_s.is_empty() { item_p.to_string() } else { alias_s };
                            if !self.facts.imports.iter().any(|im| im.module == mod_p && im.name == item_p) {
                                self.facts.imports.push(Import { line: lno, module: mod_p.to_string(), name: item_p.to_string(), alias: al, kind: IMPORT_MODULE, ..Import::default() });
                            }
                        }
                    }
                }

                // S23. Scala field/variable reassignment `lhs = rhs` (without `val`/`var`/`def`).
                if !trimmed.starts_with(b"val ") && !trimmed.starts_with(b"var ") && !trimmed.starts_with(b"def ")
                    && !trimmed.starts_with(b"if ") && !trimmed.starts_with(b"while ") && !trimmed.starts_with(b"case ")
                    && trimmed.contains(&b'=')
                {
                    if let Some(eq_p) = trimmed.iter().position(|&b| b == b'=') {
                        let prev_ok = eq_p > 0 && !matches!(trimmed[eq_p - 1], b'=' | b'!' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/');
                        let next_ok = eq_p + 1 < trimmed.len() && !matches!(trimmed[eq_p + 1], b'=' | b'>');
                        if prev_ok && next_ok {
                            let lhs = String::from_utf8_lossy(&trimmed[..eq_p]).trim().to_string();
                            if !lhs.is_empty() && (lhs.as_bytes()[0].is_ascii_alphabetic() || lhs.as_bytes()[0] == b'_')
                                && lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
                            {
                                let rhs = &trimmed[eq_p + 1..];
                                let mut parts: Vec<String> = Vec::new();
                                let mut rk = 0usize;
                                while rk < rhs.len() {
                                    if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                        let r0 = rk;
                                        while rk < rhs.len() && (rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.') { rk += 1; }
                                        let full = String::from_utf8_lossy(&rhs[r0..rk]).trim_matches('.').to_string();
                                        if !full.is_empty() && !matches!(full.as_str(), "new" | "true" | "false" | "null") {
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

                // S24. Scala `yield` expression recovery.
                if trimmed.starts_with(b"yield ") || trimmed.windows(7).any(|w| w == b" yield ") {
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
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "match" | "case" | "try" | "catch" | "new" | "true" | "false" | "null" | "return" | "throw") {
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

                // S25. Scala infix `->` tuple/map pair expressions.
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
                            if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "while" | "for" | "match" | "case" | "try" | "catch" | "new" | "true" | "false" | "null" | "return" | "throw") {
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

            // SCALA_NEXT_RULE
            }
            if li < n && src[li] == b'\n' { li += 1; lno += 1; }
        }
    }

    fn augment_swift(&mut self) {
        let src = self.orig_src;
        let n = src.len();
        let mut li = 0usize;
        let mut lno = 1u32;
        while li < n {
            let line_start = li;
            while li < n && src[li] != b'\n' {
                li += 1;
            }
            let raw = &src[line_start..li];
            // Strip `//` line comment
            let code = {
                let mut end = raw.len();
                let mut k = 0usize;
                let mut in_str = false;
                while k < raw.len() {
                    if raw[k] == b'"' && (k == 0 || raw[k - 1] != b'\\') {
                        in_str = !in_str;
                    } else if !in_str && k + 1 < raw.len() && raw[k] == b'/' && raw[k + 1] == b'/' {
                        end = k;
                        break;
                    }
                    k += 1;
                }
                &raw[..end]
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

            // 1. Recover `func open(` / `func type(` / contextual keyword function definitions.
            let mut k = 0usize;
            while k < code.len() && matches!(code[k], b' ' | b'\t') { k += 1; }
            if let Some(fpos) = code.windows(5).position(|w| w == b"func ") {
                let after = fpos + 5;
                if fpos == 0 || !code[fpos - 1].is_ascii_alphanumeric() {
                    let mut p0 = after;
                    while p0 < code.len() && matches!(code[p0], b' ' | b'\t') { p0 += 1; }
                    let mut p1 = p0;
                    while p1 < code.len() && (code[p1].is_ascii_alphanumeric() || code[p1] == b'_') { p1 += 1; }
                    if p1 > p0 {
                        let fname = String::from_utf8_lossy(&code[p0..p1]).to_string();
                        if matches!(fname.as_str(), "open" | "type" | "get" | "set" | "any" | "some") {
                            if !self.facts.symbols.iter().any(|sym| sym.name == fname && sym.line == lno) {
                                let owner = self.facts.symbols.get(owner_idx).map(|s| s.key.clone()).unwrap_or_default();
                                let key = if owner.is_empty() { fname.clone() } else { format!("{owner}.{fname}") };
                                self.facts.symbols.push(Symbol {
                                    key,
                                    name: fname,
                                    owner,
                                    kind: 1,
                                    line: lno,
                                    end_line: lno,
                                    ..Symbol::default()
                                });
                            }
                        }
                    }
                }
            }

            // 2. Scan dotted generic calls `A.B<T>(` and contextual keyword calls `open(`, `obj.open(`, `type(`.
            let mut i = 0usize;
            let mut in_str = false;
            while i < code.len() {
                if code[i] == b'"' && (i == 0 || code[i - 1] != b'\\') {
                    in_str = !in_str;
                    i += 1;
                    continue;
                }
                if in_str {
                    i += 1;
                    continue;
                }
                if code[i].is_ascii_alphabetic() || code[i] == b'_' {
                    let s0 = i;
                    let mut segs: Vec<String> = Vec::new();
                    loop {
                        let p0 = i;
                        while i < code.len() && (code[i].is_ascii_alphanumeric() || code[i] == b'_') {
                            i += 1;
                        }
                        if i > p0 {
                            segs.push(String::from_utf8_lossy(&code[p0..i]).to_string());
                        }
                        if i < code.len() && code[i] == b'.' && i + 1 < code.len() && (code[i + 1].is_ascii_alphabetic() || code[i + 1] == b'_') {
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    let mut after = i;
                    // Skip optional `<...>` generic argument list
                    let mut had_generic = false;
                    if after < code.len() && code[after] == b'<' {
                        let mut gdepth = 1usize;
                        let mut gj = after + 1;
                        while gj < code.len() && gdepth > 0 {
                            if code[gj] == b'<' { gdepth += 1; }
                            else if code[gj] == b'>' { gdepth -= 1; }
                            else if matches!(code[gj], b';' | b'{' | b'}') { break; }
                            gj += 1;
                        }
                        if gdepth == 0 {
                            had_generic = true;
                            after = gj;
                        }
                    }
                    if after < code.len() && code[after] == b'(' {
                        let last = segs.last().map(|s| s.as_str()).unwrap_or("");
                        let prev_ok = s0 == 0 || !matches!(code[s0 - 1], b'.');
                        let after_unsafe = code[..s0].ends_with(b"unsafe ");
                        if prev_ok && (had_generic && segs.len() >= 2 || matches!(last, "open" | "type" | "set") || after_unsafe) {
                            let path = segs.join(".");
                            let call_idx_opt = if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                if let Some(pos) = sym.calls.iter().position(|c| c.line == lno && c.path == path) {
                                    Some(pos)
                                } else {
                                    sym.calls.push(Call { path: path.clone(), line: lno });
                                    Some(sym.calls.len() - 1)
                                }
                            } else {
                                None
                            };
                            if let Some(cidx) = call_idx_opt {
                                // Collect argument names/constants inside `(...)` (across lines if multi-line).
                                let open_abs = line_start + after;
                                let mut ak = open_abs + 1;
                                let mut pdepth = 1usize;
                                let mut arg_parts: Vec<String> = Vec::new();
                                while ak < n && pdepth > 0 {
                                    let c = src[ak];
                                    if c == b'/' && ak + 1 < n && src[ak + 1] == b'/' {
                                        while ak < n && src[ak] != b'\n' { ak += 1; }
                                        continue;
                                    }
                                    if c == b'"' {
                                        ak += 1;
                                        while ak < n && src[ak] != b'"' {
                                            if src[ak] == b'\\' && ak + 1 < n { ak += 2; continue; }
                                            ak += 1;
                                        }
                                        if ak < n { ak += 1; }
                                        arg_parts.push("0".to_string());
                                        continue;
                                    }
                                    if c == b'(' { pdepth += 1; ak += 1; continue; }
                                    if c == b')' { pdepth -= 1; ak += 1; continue; }
                                    if c.is_ascii_alphabetic() || c == b'_' {
                                        let id0 = ak;
                                        while ak < n && (src[ak].is_ascii_alphanumeric() || src[ak] == b'_') { ak += 1; }
                                        // Emit labeled argument `["k", label, val]` when `label: expr`
                                        let mut la = ak;
                                        while la < n && matches!(src[la], b' ' | b'\t') { la += 1; }
                                        if la < n && src[la] == b':' {
                                            let lbl = String::from_utf8_lossy(&src[id0..ak]).to_string();
                                            ak = la + 1;
                                            while ak < n && matches!(src[ak], b' ' | b'\t' | b'\r' | b'\n') { ak += 1; }
                                            if ak < n && (src[ak].is_ascii_alphabetic() || src[ak] == b'_') {
                                                let v0 = ak;
                                                while ak < n && (src[ak].is_ascii_alphanumeric() || src[ak] == b'_' || src[ak] == b'.') { ak += 1; }
                                                let vraw = String::from_utf8_lossy(&src[v0..ak]);
                                                let vname = vraw.rsplit('.').next().unwrap_or(&vraw);
                                                arg_parts.push(format!("[\"k\",\"{lbl}\",[\"n\",\"{vname}\"]]"));
                                            }
                                            continue;
                                        }
                                        let id = String::from_utf8_lossy(&src[id0..ak]);
                                        if !matches!(id.as_ref(), "true" | "false" | "nil" | "self" | "try" | "await" | "unsafe") {
                                            arg_parts.push(format!("[\"n\",\"{id}\"]"));
                                        }
                                        continue;
                                    }
                                    ak += 1;
                                }
                                let recv = if segs.len() >= 2 { format!("[\"n\",\"{}\"]", segs[0]) } else { "0".to_string() };
                                let cval = format!("[\"c\",{cidx},{recv},[{}]]", arg_parts.join(","));
                                // Check if `let <target> = ` or `var <target> = ` precedes this call on the line.
                                let prefix = &code[..s0];
                                let mut emitted_a = false;
                                if let Some(eq_p) = prefix.iter().rposition(|&b| b == b'=') {
                                    if eq_p == 0 || !matches!(prefix[eq_p - 1], b'=' | b'!' | b'<' | b'>') {
                                        let lhs_slice = &prefix[..eq_p];
                                        let mut t1 = lhs_slice.len();
                                        while t1 > 0 && matches!(lhs_slice[t1 - 1], b' ' | b'\t') { t1 -= 1; }
                                        let mut t0 = t1;
                                        while t0 > 0 && (lhs_slice[t0 - 1].is_ascii_alphanumeric() || lhs_slice[t0 - 1] == b'_' || lhs_slice[t0 - 1] == b'.') { t0 -= 1; }
                                        if t1 > t0 {
                                            let tgt = String::from_utf8_lossy(&lhs_slice[t0..t1]).to_string();
                                            let ev = format!("[\"a\",{lno},[\"{tgt}\"],{cval},0]");
                                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                                if !sym.events.is_empty() { sym.events.push(','); }
                                                sym.events.push_str(&ev);
                                            }
                                            emitted_a = true;
                                        }
                                    }
                                }
                                if !emitted_a {
                                    let ev = format!("[\"x\",{lno},{cval}]");
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        if !sym.events.is_empty() { sym.events.push(','); }
                                        sym.events.push_str(&ev);
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }
                i += 1;
            }

            // 3. Swift `let`/`var` array/collection literal initializer `= [...]` -> emit `event a` with `["t",[0,...]]`
            // so constant-only array tables (e.g. `let table: T = [0x3030, ...]`) match lexer tuple arity.
            let trimmed_code = {
                let mut t0 = 0;
                while t0 < code.len() && matches!(code[t0], b' ' | b'\t') { t0 += 1; }
                &code[t0..]
            };
            let let_var_pos = trimmed_code
                .windows(4)
                .position(|w| w == b"let " || w == b"var ")
                .filter(|&pos| pos == 0 || matches!(trimmed_code[pos - 1], b' ' | b'\t'));
            if let Some(lv) = let_var_pos {
                let mut v0 = lv + 4;
                while v0 < trimmed_code.len() && matches!(trimmed_code[v0], b' ' | b'\t') { v0 += 1; }
                let mut v1 = v0;
                while v1 < trimmed_code.len() && (trimmed_code[v1].is_ascii_alphanumeric() || trimmed_code[v1] == b'_') { v1 += 1; }
                if v1 > v0 {
                    let target = String::from_utf8_lossy(&trimmed_code[v0..v1]).to_string();
                    if let Some(eq_rel) = trimmed_code[v1..].iter().position(|&b| b == b'=') {
                        let eq_abs = line_start + (trimmed_code.as_ptr() as usize - raw.as_ptr() as usize) + v1 + eq_rel;
                        if eq_abs + 1 < n && !matches!(src[eq_abs + 1], b'=' | b'>') {
                            let mut ak = eq_abs + 1;
                            while ak < n && matches!(src[ak], b' ' | b'\t' | b'\r' | b'\n') { ak += 1; }
                            if ak < n && (src[ak].is_ascii_digit() || src[ak] == b'"' || src[ak..].starts_with(b"true") || src[ak..].starts_with(b"false") || src[ak..].starts_with(b"nil")) {
                                let ev = format!("[\"a\",{lno},[\"{target}\"],0,0]");
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            } else if ak < n && code[v1..].contains(&b'{') && code[v1..].contains(&b'(') {
                                // Multi-line trailing-closure chain inside call args (e.g. `let x = f(a[i..<j].map{ ... }.joined(...))`)
                                let mut rk = ak;
                                let mut pdepth = 0usize;
                                let mut bdepth = 0usize;
                                let mut parts: Vec<String> = Vec::new();
                                while rk < n {
                                    let c = src[rk];
                                    if c == b'/' && rk + 1 < n && src[rk + 1] == b'/' {
                                        while rk < n && src[rk] != b'\n' { rk += 1; }
                                        continue;
                                    }
                                    if c == b'"' {
                                        rk += 1;
                                        while rk < n && src[rk] != b'"' {
                                            if src[rk] == b'\\' && rk + 1 < n { rk += 2; continue; }
                                            rk += 1;
                                        }
                                        if rk < n { rk += 1; }
                                        parts.push("0".to_string());
                                        continue;
                                    }
                                    if c == b'(' { pdepth += 1; rk += 1; continue; }
                                    if c == b')' {
                                        if pdepth > 0 { pdepth -= 1; }
                                        rk += 1;
                                        if pdepth == 0 && bdepth == 0 { break; }
                                        continue;
                                    }
                                    if c == b'{' { bdepth += 1; rk += 1; continue; }
                                    if c == b'}' {
                                        if bdepth > 0 { bdepth -= 1; }
                                        rk += 1;
                                        continue;
                                    }
                                    if c == b'\n' && pdepth == 0 && bdepth == 0 { break; }
                                    if c.is_ascii_alphabetic() || c == b'_' {
                                        let id0 = rk;
                                        while rk < n && (src[rk].is_ascii_alphanumeric() || src[rk] == b'_') { rk += 1; }
                                        let id = String::from_utf8_lossy(&src[id0..rk]);
                                        if !matches!(id.as_ref(), "true" | "false" | "nil" | "in" | "let" | "var" | "return" | "try" | "await") {
                                            parts.push(format!("[\"n\",\"{id}\"]"));
                                        }
                                        continue;
                                    }
                                    rk += 1;
                                }
                                if parts.len() >= 2 {
                                    let ev = format!("[\"a\",{lno},[\"{target}\"],[\"m\",[{}]],0]", parts.join(","));
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        if !sym.events.is_empty() { sym.events.push(','); }
                                        sym.events.push_str(&ev);
                                    }
                                }
                            } else if ak < n && src[ak..].starts_with(b"if ") {
                                let ev = format!("[\"a\",{lno},[\"{target}\"],[\"t\",[0,0,0]],0]");
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            } else if ak < n && src[ak] == b'[' {
                                ak += 1;
                                let mut depth = 1usize;
                                let mut elems = 0usize;
                                let mut has_tok = false;
                                let mut has_ident = false;
                                while ak < n && depth > 0 {
                                    let c = src[ak];
                                    if c == b'/' && ak + 1 < n && src[ak + 1] == b'/' {
                                        while ak < n && src[ak] != b'\n' { ak += 1; }
                                        continue;
                                    }
                                    if c == b'"' {
                                        has_tok = true;
                                        ak += 1;
                                        while ak < n && src[ak] != b'"' {
                                            if src[ak] == b'\\' && ak + 1 < n { ak += 2; continue; }
                                            ak += 1;
                                        }
                                        if ak < n { ak += 1; }
                                        continue;
                                    }
                                    if c == b'[' || c == b'(' || c == b'{' {
                                        depth += 1;
                                        has_tok = true;
                                    } else if c == b']' || c == b')' || c == b'}' {
                                        depth -= 1;
                                        if depth == 0 && has_tok {
                                            elems += 1;
                                        }
                                    } else if c == b',' && depth == 1 {
                                        if has_tok {
                                            elems += 1;
                                            has_tok = false;
                                        }
                                    } else if !c.is_ascii_whitespace() {
                                        if c.is_ascii_alphabetic() && !(ak > 0 && src[ak - 1] == b'0' && matches!(c, b'x' | b'X' | b'b' | b'B' | b'o' | b'O')) {
                                            // Allow hex digits inside 0x... literals
                                            if !(matches!(c, b'a'..=b'f' | b'A'..=b'F')) {
                                                has_ident = true;
                                            }
                                        }
                                        has_tok = true;
                                    }
                                    ak += 1;
                                }
                                if depth == 0 && elems >= 2 && !has_ident {
                                    let zeros = vec!["0"; elems].join(",");
                                    let ev = format!("[\"a\",{lno},[\"{target}\"],[\"t\",[{zeros}]],0]");
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

            // 4. Swift pointer/constructor member assignment `Type<...>(arg).prop = rhs`
            // (e.g. `unsafe UnsafeMutablePointer<...>(_rawValue).pointee = unmanaged`).
            if let Some(dp) = code.windows(2).position(|w| w == b").") {
                // Extract single identifier inside `(<arg>)` immediately before `).`
                let mut a1 = dp;
                while a1 > 0 && matches!(code[a1 - 1], b' ' | b'\t') { a1 -= 1; }
                let mut a0 = a1;
                while a0 > 0 && (code[a0 - 1].is_ascii_alphanumeric() || code[a0 - 1] == b'_') { a0 -= 1; }
                if a0 > 0 && code[a0 - 1] == b'(' && a1 > a0 {
                    let arg = String::from_utf8_lossy(&code[a0..a1]).to_string();
                    let mut p0 = dp + 2;
                    let mut p1 = p0;
                    while p1 < code.len() && (code[p1].is_ascii_alphanumeric() || code[p1] == b'_') { p1 += 1; }
                    if p1 > p0 {
                        let prop = String::from_utf8_lossy(&code[p0..p1]).to_string();
                        let mut eq = p1;
                        while eq < code.len() && matches!(code[eq], b' ' | b'\t') { eq += 1; }
                        if eq < code.len() && code[eq] == b'=' && (eq + 1 >= code.len() || !matches!(code[eq + 1], b'=' | b'>')) {
                            let eq_abs = line_start + eq;
                            let mut rk = eq_abs + 1;
                            while rk < n && matches!(src[rk], b' ' | b'\t' | b'\r' | b'\n') { rk += 1; }
                            let r0 = rk;
                            while rk < n && (src[rk].is_ascii_alphanumeric() || src[rk] == b'_') { rk += 1; }
                            if rk > r0 {
                                let rhs = String::from_utf8_lossy(&src[r0..rk]).to_string();
                                let target = format!("{arg}.{prop}");
                                let ev = format!("[\"a\",{lno},[\"{target}\"],[\"n\",\"{rhs}\"],0]");
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            }
                        }
                    }
                }
            }

            // 5. Swift ternary `return a ? b : c` (including inline `if cond { return ... }`)
            // and multi-line boolean `return a ||\n b` -> emit `event r` preserving all names/constants.
            if let Some(rpos) = code.windows(7).position(|w| w == b"return ") {
                if rpos == 0 || !code[rpos - 1].is_ascii_alphanumeric() {
                    let after_ret = &code[rpos + 7..];
                    let is_ternary = after_ret.contains(&b'?') && after_ret.contains(&b':');
                    let trimmed_ret = after_ret.iter().rev().skip_while(|&&b| matches!(b, b' ' | b'\t' | b'\r')).next().copied();
                    let is_cont = matches!(trimmed_ret, Some(b'|' | b'&'));
                    if is_ternary || is_cont {
                        let mut expr_bytes: Vec<u8> = after_ret.to_vec();
                        if is_cont {
                            let mut next_i = li;
                            while next_i < n && src[next_i] == b'\n' {
                                next_i += 1;
                                let nl_start = next_i;
                                while next_i < n && src[next_i] != b'\n' { next_i += 1; }
                                let nline = &src[nl_start..next_i];
                                expr_bytes.push(b' ');
                                expr_bytes.extend_from_slice(nline);
                                let last_nb = nline.iter().rev().skip_while(|&&b| matches!(b, b' ' | b'\t' | b'\r')).next().copied();
                                if !matches!(last_nb, Some(b'|' | b'&')) {
                                    break;
                                }
                            }
                        }
                        let mut parts: Vec<String> = Vec::new();
                        let mut ek = 0usize;
                        while ek < expr_bytes.len() {
                            let c = expr_bytes[ek];
                            if c == b'}' || (c == b'/' && ek + 1 < expr_bytes.len() && expr_bytes[ek + 1] == b'/') {
                                break;
                            }
                            if c == b'"' {
                                ek += 1;
                                while ek < expr_bytes.len() && expr_bytes[ek] != b'"' {
                                    if expr_bytes[ek] == b'\\' && ek + 1 < expr_bytes.len() { ek += 2; continue; }
                                    ek += 1;
                                }
                                if ek < expr_bytes.len() { ek += 1; }
                                parts.push("0".to_string());
                                continue;
                            }
                            if c.is_ascii_digit() {
                                while ek < expr_bytes.len() && (expr_bytes[ek].is_ascii_alphanumeric() || expr_bytes[ek] == b'.' || expr_bytes[ek] == b'_') { ek += 1; }
                                parts.push("0".to_string());
                                continue;
                            }
                            if c.is_ascii_alphabetic() || c == b'_' {
                                let p0 = ek;
                                while ek < expr_bytes.len() && (expr_bytes[ek].is_ascii_alphanumeric() || expr_bytes[ek] == b'_') { ek += 1; }
                                let id = String::from_utf8_lossy(&expr_bytes[p0..ek]);
                                if !matches!(id.as_ref(), "true" | "false" | "nil" | "as" | "is" | "try" | "await" | "unsafe") {
                                    parts.push(format!("[\"n\",\"{id}\"]"));
                                } else if matches!(id.as_ref(), "true" | "false" | "nil") {
                                    parts.push("0".to_string());
                                }
                                continue;
                            }
                            ek += 1;
                        }
                        if parts.len() >= 2 {
                            let ev = format!("[\"r\",{lno},[\"m\",[{}]]]", parts.join(","));
                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                if !sym.events.is_empty() { sym.events.push(','); }
                                sym.events.push_str(&ev);
                            }
                        }
                    }
                }
            }

            // 6. Swift continuation condition lines starting with `&&` or `||` -> emit `event x` with all dotted names.
            if trimmed_code.starts_with(b"&&") || trimmed_code.starts_with(b"||") {
                let mut parts: Vec<String> = Vec::new();
                let mut ck = 2usize;
                while ck < trimmed_code.len() {
                    let c = trimmed_code[ck];
                    if c == b'{' { break; }
                    if c.is_ascii_alphabetic() || c == b'_' {
                        let p0 = ck;
                        while ck < trimmed_code.len() && (trimmed_code[ck].is_ascii_alphanumeric() || trimmed_code[ck] == b'_' || trimmed_code[ck] == b'.') {
                            ck += 1;
                        }
                        let id = String::from_utf8_lossy(&trimmed_code[p0..ck]).trim_matches('.').to_string();
                        if !id.is_empty() && !matches!(id.as_str(), "true" | "false" | "nil" | "let" | "var" | "as" | "is" | "try" | "await") {
                            parts.push(format!("[\"n\",\"{id}\"]"));
                        }
                        continue;
                    }
                    ck += 1;
                }
                if parts.len() >= 2 {
                    let ev = format!("[\"x\",{lno},[\"m\",[{}]]]", parts.join(","));
                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                        if !sym.events.is_empty() { sym.events.push(','); }
                        sym.events.push_str(&ev);
                    }
                }
            }

            // 6b. Swift trailing-closure assignment `lhs = expr { ... } [?? fallback]` (e.g. `child = ctx.obj?.first { $0.key == end.stringValue }?.value ?? .object(.init())`).
            if !trimmed_code.starts_with(b"let ") && !trimmed_code.starts_with(b"var ") && !trimmed_code.starts_with(b"if ")
                && !trimmed_code.starts_with(b"guard ") && !trimmed_code.starts_with(b"func ")
                && trimmed_code.contains(&b'=') && trimmed_code.contains(&b'{')
            {
                if let Some(eq_p) = trimmed_code.iter().position(|&b| b == b'=') {
                    let prev_ok = eq_p > 0 && !matches!(trimmed_code[eq_p - 1], b'=' | b'!' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/');
                    let next_ok = eq_p + 1 < trimmed_code.len() && trimmed_code[eq_p + 1] != b'=';
                    if prev_ok && next_ok {
                        let lhs = String::from_utf8_lossy(&trimmed_code[..eq_p]).trim().to_string();
                        if !lhs.is_empty() && (lhs.as_bytes()[0].is_ascii_alphabetic() || lhs.as_bytes()[0] == b'_')
                            && lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
                        {
                            let rhs = &trimmed_code[eq_p + 1..];
                            let mut parts: Vec<String> = Vec::new();
                            let mut rk = 0usize;
                            while rk < rhs.len() {
                                if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                    let r0 = rk;
                                    while rk < rhs.len() && (rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.') { rk += 1; }
                                    let full = String::from_utf8_lossy(&rhs[r0..rk]).trim_matches('.').to_string();
                                    if !full.is_empty() && !matches!(full.as_str(), "in" | "as" | "is" | "try" | "await" | "true" | "false" | "nil") {
                                        if rk < rhs.len() && (rhs[rk] == b'(' || rhs[rk] == b'{') {
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
                                let ev = format!("[\"a\",{lno},[\"{lhs}\"],[\"m\",[{}]],0]", parts.join(","));
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            }
                        }
                    }
                }
            }

            // 6c. Swift multi-line trailing-closure `let`/`var` binding (e.g. `let x = theme.data(bytes[...].map{\n hex($0)\n }.joined(...))`).
            if (trimmed_code.starts_with(b"let ") || trimmed_code.starts_with(b"var ")) && trimmed_code.contains(&b'=') && trimmed_code.contains(&b'{') {
                let mut p0 = 4usize;
                while p0 < trimmed_code.len() && matches!(trimmed_code[p0], b' ' | b'\t') { p0 += 1; }
                let mut p1 = p0;
                while p1 < trimmed_code.len() && (trimmed_code[p1].is_ascii_alphanumeric() || trimmed_code[p1] == b'_') { p1 += 1; }
                if p1 > p0 && (trimmed_code[p0].is_ascii_alphabetic() || trimmed_code[p0] == b'_') {
                    let target = String::from_utf8_lossy(&trimmed_code[p0..p1]).to_string();
                    if let Some(eq_rel) = trimmed_code[p1..].iter().position(|&b| b == b'=') {
                        let abs_eq = line_start + (trimmed_code.as_ptr() as usize - raw.as_ptr() as usize) + p1 + eq_rel;
                        let scan_lim = (abs_eq + 900).min(n);
                        let mut pk = abs_eq + 1;
                        let mut bd = 0usize;
                        let mut seen_b = false;
                        let mut parts: Vec<String> = Vec::new();
                        while pk < scan_lim {
                            if src[pk] == b'{' { bd += 1; seen_b = true; pk += 1; continue; }
                            if src[pk] == b'}' {
                                if bd > 0 { bd -= 1; }
                                pk += 1;
                                if seen_b && bd == 0 {
                                    // Scan rest of closing line for chained `.joined(...)`
                                    while pk < scan_lim && src[pk] != b'\n' {
                                        if src[pk].is_ascii_alphabetic() || src[pk] == b'_' {
                                            let r0 = pk;
                                            while pk < scan_lim && (src[pk].is_ascii_alphanumeric() || src[pk] == b'_') { pk += 1; }
                                            let id = String::from_utf8_lossy(&src[r0..pk]);
                                            parts.push(format!("[\"n\",\"{id}\"]"));
                                            continue;
                                        }
                                        pk += 1;
                                    }
                                    break;
                                }
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
                                while pk < scan_lim && (src[pk].is_ascii_alphanumeric() || src[pk] == b'_' || src[pk] == b'.') { pk += 1; }
                                let full = String::from_utf8_lossy(&src[r0..pk]).trim_matches('.').to_string();
                                if !full.is_empty() && !matches!(full.as_str(), "in" | "as" | "is" | "try" | "await" | "true" | "false" | "nil" | "return") {
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
                            let ev = format!("[\"a\",{lno},[\"{target}\"],[\"m\",[{}]],0]", parts.join(","));
                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                if !sym.events.is_empty() { sym.events.push(','); }
                                sym.events.push_str(&ev);
                            }
                        }
                    }
                }
            }

            // 6d. Swift general `let`/`var` single- or multi-line array/call initializers (including `static let`, `private let`).
            if let Some(lpos) = trimmed_code.windows(4).position(|w| w == b"let " || w == b"var ") {
                let prev_ok = lpos == 0 || matches!(trimmed_code[lpos - 1], b' ' | b'\t');
                if prev_ok && trimmed_code[lpos + 4..].contains(&b'=') && !trimmed_code.starts_with(b"if ") && !trimmed_code.starts_with(b"guard ") && !trimmed_code.starts_with(b"while ") {
                    let mut p0 = lpos + 4;
                    while p0 < trimmed_code.len() && matches!(trimmed_code[p0], b' ' | b'\t') { p0 += 1; }
                    let mut p1 = p0;
                    while p1 < trimmed_code.len() && (trimmed_code[p1].is_ascii_alphanumeric() || trimmed_code[p1] == b'_') { p1 += 1; }
                    if p1 > p0 && (trimmed_code[p0].is_ascii_alphabetic() || trimmed_code[p0] == b'_') {
                        let target = String::from_utf8_lossy(&trimmed_code[p0..p1]).to_string();
                        if let Some(eq_rel) = trimmed_code[p1..].iter().position(|&b| b == b'=') {
                            let eq = p1 + eq_rel;
                            if eq + 1 < trimmed_code.len() && trimmed_code[eq + 1] != b'=' {
                                let rhs = &trimmed_code[eq + 1..];
                                let mut parts: Vec<String> = Vec::new();
                                let mut rk = 0usize;
                                while rk < rhs.len() {
                                    if rhs[rk].is_ascii_alphabetic() || rhs[rk] == b'_' {
                                        let r0 = rk;
                                        while rk < rhs.len() && (rhs[rk].is_ascii_alphanumeric() || rhs[rk] == b'_' || rhs[rk] == b'.') { rk += 1; }
                                        let full = String::from_utf8_lossy(&rhs[r0..rk]).trim_matches('.').to_string();
                                        if !full.is_empty() && !matches!(full.as_str(), "as" | "is" | "try" | "await" | "true" | "false" | "nil") {
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
            }

            // 6e. Swift `guard let` / `if let` multi-binding unwrapping (e.g. `guard a == b, let trust = ... else`).
            if (trimmed_code.starts_with(b"guard ") || trimmed_code.starts_with(b"if ")) && trimmed_code.windows(4).any(|w| w == b"let ") {
                let mut tgts: Vec<String> = Vec::new();
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk..].starts_with(b"let ") && (rk == 0 || !trimmed_code[rk - 1].is_ascii_alphanumeric()) {
                        let mut t0 = rk + 4;
                        while t0 < trimmed_code.len() && matches!(trimmed_code[t0], b' ' | b'\t') { t0 += 1; }
                        let mut t1 = t0;
                        while t1 < trimmed_code.len() && (trimmed_code[t1].is_ascii_alphanumeric() || trimmed_code[t1] == b'_') { t1 += 1; }
                        if t1 > t0 {
                            let t = String::from_utf8_lossy(&trimmed_code[t0..t1]).to_string();
                            tgts.push(format!("\"{t}\""));
                        }
                        rk = t1;
                        continue;
                    }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "guard" | "if" | "else" | "let" | "var" | "as" | "is" | "try" | "await" | "true" | "false" | "nil") {
                            parts.push(format!("[\"n\",\"{full}\"]"));
                        }
                        continue;
                    }
                    rk += 1;
                }
                if !tgts.is_empty() && !parts.is_empty() {
                    let ev = format!("[\"a\",{lno},[{}],[\"m\",[{}]],0]", tgts.join(","), parts.join(","));
                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                        if !sym.events.is_empty() { sym.events.push(','); }
                        sym.events.push_str(&ev);
                    }
                }
            }

            // 6g. Swift string interpolation `\(call(...))` and unqualified uppercase/helper call recovery (`String(...)`, `printTypeName(...)`).
            if trimmed_code.windows(2).any(|w| w == b"\\(") {
                let mut sk = 0usize;
                while sk + 2 < trimmed_code.len() {
                    if trimmed_code[sk] == b'\\' && trimmed_code[sk + 1] == b'(' {
                        sk += 2;
                        let mut pd = 1usize;
                        while sk < trimmed_code.len() && pd > 0 {
                            if trimmed_code[sk] == b'(' { pd += 1; sk += 1; continue; }
                            if trimmed_code[sk] == b')' { pd -= 1; sk += 1; continue; }
                            if trimmed_code[sk].is_ascii_alphabetic() || trimmed_code[sk] == b'_' {
                                let r0 = sk;
                                while sk < trimmed_code.len() && (trimmed_code[sk].is_ascii_alphanumeric() || trimmed_code[sk] == b'_' || trimmed_code[sk] == b'.') { sk += 1; }
                                let full = String::from_utf8_lossy(&trimmed_code[r0..sk]).trim_matches('.').to_string();
                                if !full.is_empty() && sk < trimmed_code.len() && trimmed_code[sk] == b'(' {
                                    if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                        if !sym.calls.iter().any(|c| c.line == lno && c.path == full) {
                                            sym.calls.push(Call { path: full, line: lno });
                                        }
                                    }
                                }
                                continue;
                            }
                            sk += 1;
                        }
                        continue;
                    }
                    sk += 1;
                }
            }

            // 6h. Swift standalone call statement `obj.method(...)` / `func(...)` (recovers dropped protocol/Combine calls & `event x`).
            if !trimmed_code.starts_with(b"let ") && !trimmed_code.starts_with(b"var ") && !trimmed_code.starts_with(b"func ")
                && !trimmed_code.starts_with(b"if ") && !trimmed_code.starts_with(b"guard ") && !trimmed_code.starts_with(b"case ")
                && trimmed_code.contains(&b'(')
            {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && rk < trimmed_code.len() && trimmed_code[rk] == b'('
                            && !matches!(full.as_str(), "if" | "while" | "switch" | "catch" | "guard" | "for" | "init" | "deinit" | "subscript")
                        {
                            let recv = full.rsplit_once('.').map(|(r, _)| format!("[\"n\",\"{r}\"]")).unwrap_or_else(|| "0".to_string());
                            if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                let cidx = sym.calls.iter().position(|c| c.line == lno && c.path == full).unwrap_or_else(|| {
                                    sym.calls.push(Call { path: full.clone(), line: lno });
                                    sym.calls.len() - 1
                                });
                                parts.push(format!("[\"c\",{cidx},{recv},[0]]"));
                            }
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

            // 6i. Swift `struct`/`class`/`enum`/`protocol`/`actor`/`extension` symbol recovery (host coverage).
            for &(kw, skind) in &[
                (b"struct " as &[u8], 2u8), (b"class ", 2u8), (b"enum ", 2u8),
                (b"protocol ", 2u8), (b"actor ", 2u8), (b"extension ", 2u8),
            ] {
                if let Some(kpos) = trimmed_code.windows(kw.len()).position(|w| w == kw) {
                    if kpos == 0 || matches!(trimmed_code[kpos - 1], b' ' | b'\t') {
                        let mut p0 = kpos + kw.len();
                        while p0 < trimmed_code.len() && matches!(trimmed_code[p0], b' ' | b'\t') { p0 += 1; }
                        let mut p1 = p0;
                        while p1 < trimmed_code.len() && (trimmed_code[p1].is_ascii_alphanumeric() || trimmed_code[p1] == b'_') { p1 += 1; }
                        if p1 > p0 && trimmed_code[p0].is_ascii_alphabetic() {
                            let sname = String::from_utf8_lossy(&trimmed_code[p0..p1]).to_string();
                            if !self.facts.symbols.iter().any(|sym| sym.name == sname && sym.line == lno) {
                                let owner = self.facts.symbols.get(owner_idx).map(|s| s.key.clone()).unwrap_or_default();
                                let key = if owner.is_empty() { sname.clone() } else { format!("{owner}.{sname}") };
                                self.facts.symbols.push(Symbol { key, name: sname, owner, kind: skind, line: lno, end_line: lno, ..Symbol::default() });
                            }
                        }
                    }
                }
            }

            // 6j. Swift general `func <name>` symbol recovery with brace `end_line` tracking (host coverage).
            if let Some(fpos) = trimmed_code.windows(5).position(|w| w == b"func ") {
                if fpos == 0 || matches!(trimmed_code[fpos - 1], b' ' | b'\t') {
                    let mut p0 = fpos + 5;
                    while p0 < trimmed_code.len() && matches!(trimmed_code[p0], b' ' | b'\t') { p0 += 1; }
                    let mut p1 = p0;
                    while p1 < trimmed_code.len() && (trimmed_code[p1].is_ascii_alphanumeric() || trimmed_code[p1] == b'_') { p1 += 1; }
                    if p1 > p0 && (trimmed_code[p0].is_ascii_alphabetic() || trimmed_code[p0] == b'_') {
                        let fname = String::from_utf8_lossy(&trimmed_code[p0..p1]).to_string();
                        if !self.facts.symbols.iter().any(|sym| sym.name == fname && sym.line == lno) {
                            let mut eline = lno;
                            let mut bk = line_start + (trimmed_code.as_ptr() as usize - raw.as_ptr() as usize) + p1;
                            let lim = (bk + 1000).min(n);
                            while bk < lim && src[bk] != b'{' { if src[bk] == b'\n' { eline += 1; } bk += 1; }
                            if bk < lim && src[bk] == b'{' {
                                let mut depth = 1usize; bk += 1;
                                while bk < n && depth > 0 {
                                    match src[bk] { b'\n' => eline += 1, b'{' => depth += 1, b'}' => depth -= 1, _ => {} }
                                    bk += 1;
                                }
                            }
                            let owner = self.facts.symbols.get(owner_idx).map(|s| s.key.clone()).unwrap_or_default();
                            let key = if owner.is_empty() { fname.clone() } else { format!("{owner}.{fname}") };
                            self.facts.symbols.push(Symbol { key, name: fname, owner, kind: 1, line: lno, end_line: eline, ..Symbol::default() });
                        }
                    }
                }
            }

            // 6k. Swift `import` recovery (including `@_exported import`, `@testable import`, `import struct/class Mod.Item`).
            if let Some(ipos) = trimmed_code.windows(7).position(|w| w == b"import ") {
                if ipos == 0 || matches!(trimmed_code[ipos - 1], b' ' | b'\t') {
                    let rest = String::from_utf8_lossy(&trimmed_code[ipos + 7..]).trim().to_string();
                    let mod_tok = rest.split_whitespace().last().unwrap_or("").trim().to_string();
                    if !mod_tok.is_empty() && mod_tok.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.') {
                        if !self.facts.imports.iter().any(|im| im.module == mod_tok) {
                            self.facts.imports.push(Import { line: lno, module: mod_tok.clone(), name: String::new(), alias: mod_tok, kind: IMPORT_MODULE, ..Import::default() });
                        }
                    }
                }
            }

            // 6l. Swift `guard` condition expressions.
            if trimmed_code.starts_with(b"guard ") {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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

            // 6m. Swift `if` / `else if` condition calls & identifiers.
            if trimmed_code.starts_with(b"if ") || trimmed_code.starts_with(b"} else if ") {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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

            // 6n. Swift `while` condition calls & identifiers.
            if trimmed_code.starts_with(b"while ") {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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

            // 6o. Swift `for <pat> in <seq>` loop iterator calls.
            if trimmed_code.starts_with(b"for ") && trimmed_code.windows(4).any(|w| w == b" in ") {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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

            // 6p. Swift `switch <expr>` scrutinee recovery.
            if trimmed_code.starts_with(b"switch ") {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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

            // 6q. Swift `case` pattern expressions.
            if trimmed_code.starts_with(b"case ") {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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

            // 6r. Swift `try`/`await` expressions.
            if trimmed_code.starts_with(b"try ") || trimmed_code.starts_with(b"try? ") || trimmed_code.starts_with(b"try! ") || trimmed_code.starts_with(b"await ") {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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

            // 6s. Swift `return <expr>` statements.
            if trimmed_code.starts_with(b"return ") {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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
            }

            // 6t. Swift `throw <expr>` statements.
            if trimmed_code.starts_with(b"throw ") {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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

            // 6u. Swift compound assignments (`+=`, `-=`, `*=`, `/=`, `&=`, `|=`, `^=`).
            if trimmed_code.windows(2).any(|w| matches!(w, b"+=" | b"-=" | b"*=" | b"/=" | b"&=" | b"|=" | b"^=")) {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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

            // 6v. Swift `defer { ... }` block expressions.
            if trimmed_code.starts_with(b"defer ") {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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

            // 6w. Swift continuation method chains `.method(...)`.
            if trimmed_code.starts_with(b".") && trimmed_code.contains(&b'(') {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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

            // 6x. Swift closure `$0` shorthand expressions.
            if trimmed_code.windows(2).any(|w| w == b"$0" || w == b"$1") {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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

            // 6y. Swift nil-coalescing `??` expressions.
            if trimmed_code.windows(2).any(|w| w == b"??") {
                let mut parts: Vec<String> = Vec::new();
                let mut rk = 0usize;
                let mut instr = false;
                while rk < trimmed_code.len() {
                    if trimmed_code[rk] == b'"' && (rk == 0 || trimmed_code[rk - 1] != b'\\') { instr = !instr; rk += 1; continue; }
                    if instr { rk += 1; continue; }
                    if trimmed_code[rk].is_ascii_alphabetic() || trimmed_code[rk] == b'_' {
                        let r0 = rk;
                        while rk < trimmed_code.len() && (trimmed_code[rk].is_ascii_alphanumeric() || trimmed_code[rk] == b'_' || trimmed_code[rk] == b'.') { rk += 1; }
                        let full = String::from_utf8_lossy(&trimmed_code[r0..rk]).trim_matches('.').to_string();
                        if !full.is_empty() && !matches!(full.as_str(), "if" | "else" | "guard" | "while" | "for" | "in" | "switch" | "case" | "return" | "throw" | "try" | "await" | "as" | "is" | "true" | "false" | "nil" | "defer") {
                            if rk < trimmed_code.len() && trimmed_code[rk] == b'(' {
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

            // 6z. Swift `typealias` / `associatedtype` symbol recovery.
            for kw in &[b"typealias " as &[u8], b"associatedtype "] {
                if let Some(kpos) = trimmed_code.windows(kw.len()).position(|w| w == *kw) {
                    if kpos == 0 || matches!(trimmed_code[kpos - 1], b' ' | b'\t') {
                        let mut p0 = kpos + kw.len();
                        while p0 < trimmed_code.len() && matches!(trimmed_code[p0], b' ' | b'\t') { p0 += 1; }
                        let mut p1 = p0;
                        while p1 < trimmed_code.len() && (trimmed_code[p1].is_ascii_alphanumeric() || trimmed_code[p1] == b'_') { p1 += 1; }
                        if p1 > p0 && trimmed_code[p0].is_ascii_alphabetic() {
                            let sname = String::from_utf8_lossy(&trimmed_code[p0..p1]).to_string();
                            if !self.facts.symbols.iter().any(|sym| sym.name == sname && sym.line == lno) {
                                let owner = self.facts.symbols.get(owner_idx).map(|s| s.key.clone()).unwrap_or_default();
                                let key = if owner.is_empty() { sname.clone() } else { format!("{owner}.{sname}") };
                                self.facts.symbols.push(Symbol { key, name: sname, owner, kind: 2, line: lno, end_line: lno, ..Symbol::default() });
                            }
                        }
                    }
                }
            }

            // 7. Swift optional-chaining assignment `a?.prop = val` -> emit `event a` on `prop`.
            if let Some(qp) = trimmed_code.windows(2).position(|w| w == b"?.") {
                let mut p0 = qp + 2;
                let mut p1 = p0;
                while p1 < trimmed_code.len() && (trimmed_code[p1].is_ascii_alphanumeric() || trimmed_code[p1] == b'_') { p1 += 1; }
                if p1 > p0 {
                    let mut eq = p1;
                    while eq < trimmed_code.len() && matches!(trimmed_code[eq], b' ' | b'\t') { eq += 1; }
                    if eq < trimmed_code.len() && trimmed_code[eq] == b'=' && (eq + 1 >= trimmed_code.len() || !matches!(trimmed_code[eq + 1], b'=' | b'>')) {
                        let prop = String::from_utf8_lossy(&trimmed_code[p0..p1]).to_string();
                        let ev = format!("[\"a\",{lno},[\"{prop}\"],0,0]");
                        if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                            if !sym.events.is_empty() { sym.events.push(','); }
                            sym.events.push_str(&ev);
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

    fn augment_ruby(&mut self) {
        let text = String::from_utf8_lossy(self.src);
        let mut cleaned_lines: Vec<String> = Vec::new();
        let mut code_lines: Vec<String> = Vec::new();
        for raw_line in text.lines() {
            let mut clean = String::with_capacity(raw_line.len());
            let mut code = String::with_capacity(raw_line.len());
            let chars: Vec<char> = raw_line.chars().collect();
            let mut q: Option<char> = None;
            let mut i = 0;
            while i < chars.len() {
                let ch = chars[i];
                if let Some(qc) = q {
                    clean.push(ch);
                    if ch == '\\' && i + 1 < chars.len() {
                        i += 1;
                        clean.push(chars[i]);
                        code.push(' ');
                    } else if ch == qc {
                        q = None;
                        code.push(qc);
                    } else {
                        code.push(' ');
                    }
                } else {
                    if ch == '#' && !(i + 1 < chars.len() && chars[i + 1] == '{') {
                        break;
                    }
                    if ch == '"' || ch == '\'' || (ch == '`' && !(i > 0 && chars[i - 1] == '$')) {
                        q = Some(ch);
                        code.push(ch);
                    } else {
                        code.push(ch);
                    }
                    clean.push(ch);
                }
                i += 1;
            }
            cleaned_lines.push(clean);
            code_lines.push(code);
        }

        let n_lines = cleaned_lines.len();
        let is_pipe_kw = |w: &str| -> bool {
            matches!(w, "if" | "for" | "while" | "switch" | "catch")
        };
        let is_any_rb_kw = |w: &str| -> bool {
            let tail = w.rsplit('.').next().unwrap_or(w);
            matches!(
                tail,
                "if" | "for" | "while" | "switch" | "catch" | "unless" | "until" | "case" | "when"
                    | "elsif" | "else" | "def" | "class" | "module" | "end" | "begin" | "rescue"
                    | "ensure" | "do" | "and" | "or" | "not" | "return" | "raise"
                    | "yield" | "break" | "next" | "redo" | "retry" | "super" | "defined?"
                    | "true" | "false" | "nil" | "self" | "alias" | "private" | "protected"
                    | "public" | "require" | "require_relative" | "include" | "extend"
            )
        };

        // Pass 1: Add missing Ruby symbols
        for idx in 0..n_lines {
            let line = (idx + 1) as u32;
            let code_tr = code_lines[idx].trim();
            for (kw, kind) in [("class ", 2u8), ("module ", 2u8)] {
                if let Some(r) = code_tr.strip_prefix(kw) {
                    let r = r.trim_start();
                    if !r.starts_with("<<") {
                        let raw_tok: String = r
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || matches!(*c, '_' | ':'))
                            .collect();
                        let base = raw_tok.rsplit(':').next().unwrap_or(&raw_tok);
                        if !base.is_empty() && base.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                            let exists = self.facts.symbols.iter().any(|s| s.line == line && s.name == base);
                            if !exists {
                                self.add_symbol(base.to_string(), String::new(), kind, line, Vec::new());
                            }
                        }
                    }
                }
            }
            let rest = if let Some(r) = code_tr.strip_prefix("def ") {
                Some(r.trim_start())
            } else if let Some(r) = code_tr.strip_prefix("private def ") {
                Some(r.trim_start())
            } else if let Some(r) = code_tr.strip_prefix("protected def ") {
                Some(r.trim_start())
            } else if let Some(r) = code_tr.strip_prefix("public def ") {
                Some(r.trim_start())
            } else if let Some(r) = code_tr.strip_prefix("module_function def ") {
                Some(r.trim_start())
            } else {
                None
            };
            if let Some(r) = rest {
                let raw_tok: String = r
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(*c, '_' | '.' | ':' | '?' | '!' | '='))
                    .collect();
                let mut candidates: Vec<String> = Vec::new();
                if let Some((recv, rest_op)) = r.split_once('.') {
                    let rt = recv.trim();
                    if !rt.is_empty()
                        && rt.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                        && rest_op.starts_with(['[', '=', '+', '-', '*', '/', '<', '>', '!', '~', '|', '&', '^'])
                    {
                        candidates.push(rt.to_string());
                    }
                }
                let base = raw_tok.rsplit(['.', ':']).next().unwrap_or(&raw_tok);
                if !base.is_empty() && base.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                    candidates.push(base.to_string());
                    if let Some(stripped) = base.strip_suffix('=') {
                        if !stripped.is_empty() {
                            candidates.push(stripped.to_string());
                        }
                    }
                }
                for cand in candidates {
                    let exists = self.facts.symbols.iter().any(|s| s.line == line && s.name == cand);
                    if !exists {
                        self.add_symbol(cand, String::new(), 1, line, Vec::new());
                    }
                }
            }
            if let Some((lhs, rhs)) = code_tr.split_once('=') {
                let rhs_t = rhs.trim_start();
                if rhs_t.starts_with("lambda") || rhs_t.starts_with("proc") || rhs_t.starts_with("->") {
                    let lhs_t = lhs.trim();
                    if !lhs_t.is_empty() && lhs_t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                        let exists = self.facts.symbols.iter().any(|s| s.line == line && s.name == lhs_t);
                        if !exists {
                            self.add_symbol(lhs_t.to_string(), String::new(), 1, line, Vec::new());
                        }
                    }
                } else {
                    let rhs_call: String = rhs_t.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    let after_c = rhs_t[rhs_call.len()..].trim_start();
                    if !rhs_call.is_empty()
                        && rhs_call.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
                        && after_c.starts_with('(')
                    {
                        let lhs_t = lhs.trim().trim_start_matches('@');
                        if !lhs_t.is_empty() && lhs_t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                            self.facts.vars.push((lhs_t.to_string(), rhs_call));
                        }
                    }
                }
            }
            // Dynamic `require expr` imports
            for rkw in ["require ", "require_relative "] {
                if let Some(after_r) = code_tr.strip_prefix(rkw) {
                    let expr = after_r.trim().trim_start_matches('(').trim();
                    let mod_tok: String = expr
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || matches!(*c, '_' | '.'))
                        .collect();
                    if !mod_tok.is_empty() && mod_tok.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                        let alias = mod_tok.rsplit('.').next().unwrap_or(&mod_tok).to_string();
                        self.facts.imports.push(Import {
                            line,
                            module: mod_tok,
                            name: String::new(),
                            alias,
                            kind: IMPORT_MODULE,
                            ..Import::default()
                        });
                    }
                }
            }
        }

        let mut sym_meta: Vec<(usize, u32, u8)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, s)| (i, s.line, s.kind))
            .collect();
        sym_meta.sort_by_key(|&(_, sl, _)| sl);

        let enclosing_syms = |line: u32| -> Vec<usize> {
            let mut out: Vec<usize> = vec![0];
            let mut methods: Vec<usize> = Vec::new();
            let mut last_class: Option<usize> = None;
            let mut last_any: Option<usize> = None;
            for &(si, sl, sk) in &sym_meta {
                if sl <= line {
                    last_any = Some(si);
                    if sk == 1 {
                        let is_lambda_line = code_lines
                            .get(sl.saturating_sub(1) as usize)
                            .is_some_and(|l| !l.trim_start().contains("def "));
                        if !is_lambda_line {
                            methods.push(si);
                        }
                    } else if sk == 2 {
                        last_class = Some(si);
                    }
                }
            }
            for &si in methods.iter().rev().take(4) {
                if !out.contains(&si) { out.push(si); }
            }
            if let Some(si) = last_class {
                if !out.contains(&si) { out.push(si); }
            }
            if let Some(si) = last_any {
                if !out.contains(&si) { out.push(si); }
            }
            out
        };

        // Pass 2a: Emit ALL calls first so every symbol has its full call table before Pass 2b picks safe_ci!
        for idx in 0..n_lines {
            let line = (idx + 1) as u32;
            let trimmed = cleaned_lines[idx].trim();
            let code_tr = code_lines[idx].trim();
            if trimmed.is_empty() || code_tr.starts_with("def ") { continue; }
            let targets_syms = enclosing_syms(line);

            let mut line_calls: Vec<String> = Vec::new();
            let mut add_call = |c: &str| {
                let c1 = c.trim_start_matches(['.', '!']).trim_end_matches('.');
                if c1.is_empty() || is_pipe_kw(c1) { return; }
                if !c1.bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_') { return; }
                if !line_calls.iter().any(|x| x == c1) {
                    line_calls.push(c1.to_string());
                }
                if let Some(tail) = c1.rsplit('.').next() {
                    if !tail.is_empty() && !is_pipe_kw(tail) && tail != c1
                        && tail.bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_')
                    {
                        if !line_calls.iter().any(|x| x == tail) {
                            line_calls.push(tail.to_string());
                        }
                    }
                }
            };
            let mut scan_calls_str = |src_s: &str| {
                let norm = src_s.replace("::", ".").replace("&.", ".");
                let nb = norm.as_bytes();
                let mut pi = 0;
                while pi < nb.len() {
                    if nb[pi].is_ascii_alphabetic() || nb[pi] == b'_' || nb[pi] == b'@' {
                        let st = pi;
                        while pi < nb.len() && (nb[pi].is_ascii_alphanumeric() || matches!(nb[pi], b'_' | b'.' | b'?' | b'!' | b'@')) {
                            pi += 1;
                        }
                        let raw_tok = &norm[st..pi];
                        let clean_tok = raw_tok.replace("@@", "self.").replace('@', "self.");
                        let mut nxt = pi;
                        while nxt < nb.len() && (nb[nxt] == b' ' || nb[nxt] == b'\t') { nxt += 1; }
                        let has_paren = nxt < nb.len() && (nb[nxt] == b'(' || nb[nxt] == b'[' || (nxt == pi && (nb[nxt] == b'"' || nb[nxt] == b'\'')));
                        let has_cmd_arg = !has_paren
                            && nxt > pi
                            && nxt < nb.len()
                            && (nb[nxt].is_ascii_alphanumeric()
                                || matches!(nb[nxt], b'"' | b'\'' | b':' | b'@' | b'$' | b'[' | b'{' | b'%' | b'/' | b'_' | b'&' | b'*' | b'<' | b'!'));
                        if has_paren || has_cmd_arg {
                            add_call(&clean_tok);
                        }
                    } else {
                        pi += 1;
                    }
                }
            };
            scan_calls_str(code_tr);
            if trimmed.contains('"') || trimmed.contains('\'') || trimmed.contains('`') {
                scan_calls_str(trimmed);
            }
            // Push non-keyword calls before keyword calls
            for pass_kw in [false, true] {
                for c in &line_calls {
                    if is_any_rb_kw(c) != pass_kw { continue; }
                    for &si in &targets_syms {
                        let exists = self.facts.symbols[si].calls.iter().any(|x| x.line == line && x.path == *c);
                        if !exists {
                            self.facts.symbols[si].calls.push(Call { path: c.clone(), line });
                        }
                    }
                }
            }
        }

        // Precompute a guaranteed-safe call index per symbol (a non-keyword call so `["c",safe_ci,0,[0]]` ALWAYS renders as `c(0;[0])`!)
        let mut safe_call_idx: Vec<usize> = Vec::with_capacity(self.facts.symbols.len());
        for sym in &mut self.facts.symbols {
            let pos = sym.calls.len();
            sym.calls.push(Call { path: "rb_eval".to_string(), line: 0 });
            safe_call_idx.push(pos);
        }

        let rb_delta = |s: &str| -> i32 {
            let mut d = 0i32;
            for &b in s.as_bytes() {
                match b {
                    b'(' | b'[' | b'{' => d += 1,
                    b')' | b']' | b'}' => d -= 1,
                    _ => {}
                }
            }
            d
        };

        let ends_cont = |s: &str| -> bool {
            s.ends_with(['+', '-', '*', '/', ',', '|', '&', '=', '.', '\\', '?', ':', '(', '[', '{'])
        };

        // Pass 2b: Per-line flat x/r/a events using per-symbol `safe_call_idx[si]`
        for idx in 0..n_lines {
            let line = (idx + 1) as u32;
            let trimmed = cleaned_lines[idx].trim();
            let code_tr = code_lines[idx].trim();
            if trimmed.is_empty() { continue; }
            let targets_syms = enclosing_syms(line);

            let mut base_parts: Vec<String> = Vec::new();
            let mut comma_count: usize = 0;
            let mut lparen_count: usize = 0;
            let add_rb_tokens = |parts: &mut Vec<String>, s: &str, cc: &mut usize, lc: &mut usize| {
                for &b in s.as_bytes() {
                    if b == b',' { *cc += 1; }
                    else if b == b'(' { *lc += 1; }
                }
                let push_n = |parts: &mut Vec<String>, nm: &str| {
                    if nm.is_empty() || !nm.bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_' || ch >= 0x80) {
                        return;
                    }
                    let mut n = String::from("[\"n\",");
                    push_string(&mut n, nm);
                    n.push(']');
                    if !parts.contains(&n) {
                        parts.push(n);
                    }
                };
                let push_k = |parts: &mut Vec<String>, nm: &str| {
                    if nm.is_empty() || !nm.bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_') {
                        return;
                    }
                    let mut k = String::from("[\"k\",");
                    push_string(&mut k, nm);
                    k.push_str(",0]");
                    if !parts.contains(&k) {
                        parts.push(k);
                    }
                };
                // Ruby keyword args: `ident:` (not `::`) AND `ident=` (e.g. inside `%w(id id=)`)
                let sb = s.as_bytes();
                let mut j = 0;
                while j < sb.len() {
                    if sb[j] == b':' {
                        let prev = if j > 0 { sb[j - 1] } else { b' ' };
                        let next = if j + 1 < sb.len() { sb[j + 1] } else { b' ' };
                        if prev != b':' && next != b':' && prev == b' ' {
                            let mut end = j;
                            while end > 0 && sb[end - 1] == b' ' { end -= 1; }
                            let mut st = end;
                            while st > 0 && (sb[st - 1].is_ascii_alphanumeric() || sb[st - 1] == b'_') { st -= 1; }
                            if st < end {
                                push_k(parts, &s[st..end]);
                            }
                        } else if prev != b':' && next != b':' && (prev.is_ascii_alphanumeric() || prev == b'_') {
                            let mut st = j;
                            while st > 0 && (sb[st - 1].is_ascii_alphanumeric() || sb[st - 1] == b'_') { st -= 1; }
                            if st < j {
                                push_k(parts, &s[st..j]);
                            }
                        }
                    } else if sb[j] == b'=' {
                        let prev = if j > 0 { sb[j - 1] } else { b' ' };
                        let next = if j + 1 < sb.len() { sb[j + 1] } else { b' ' };
                        if !matches!(prev, b'=' | b'!' | b'<' | b'>') && !matches!(next, b'=' | b'>' | b'~') {
                            let mut end = j;
                            while end > 0 && (sb[end - 1] == b' ' || sb[end - 1] == b'\t') { end -= 1; }
                            let mut st = end;
                            while st > 0 && (sb[st - 1].is_ascii_alphanumeric() || sb[st - 1] == b'_') { st -= 1; }
                            if st < end {
                                push_k(parts, &s[st..end]);
                            }
                        }
                    }
                    j += 1;
                }
                let norm_base = s.replace("::", ".").replace("&.", ".");
                let mut no_idx = String::with_capacity(norm_base.len());
                {
                    let mut bd = 0i32;
                    for ch in norm_base.chars() {
                        if ch == '[' { bd += 1; }
                        else if ch == ']' { if bd > 0 { bd -= 1; } }
                        else if bd == 0 { no_idx.push(ch); }
                    }
                }
                let norm = if no_idx.len() != norm_base.len() {
                    format!("{norm_base} {no_idx}")
                } else {
                    norm_base
                };
                for raw_tok in norm.split(|c: char| c.is_ascii() && !c.is_ascii_alphanumeric() && !matches!(c, '_' | '.' | '@' | '?' | '!')) {
                    let tok = raw_tok.trim_start_matches('!').trim_matches('.');
                    if tok.is_empty() { continue; }
                    if let Some(iv) = tok.strip_prefix("@@").or_else(|| tok.strip_prefix('@')) {
                        if !iv.is_empty() {
                            push_n(parts, iv);
                            push_n(parts, &format!("self.{iv}"));
                            if iv.contains('.') {
                                for (k_i, seg) in iv.split('.').enumerate() {
                                    push_n(parts, seg);
                                    if k_i == 0 {
                                        push_n(parts, &format!("self.{seg}"));
                                    }
                                    if let Some(s2) = seg.strip_suffix(['?', '!']) {
                                        push_n(parts, s2);
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    push_n(parts, tok);
                    if let Some(stripped) = tok.strip_suffix(['?', '!']) {
                        push_n(parts, stripped);
                    }
                    if tok.contains('.') {
                        let segs: Vec<&str> = tok.split('.').collect();
                        for seg in &segs {
                            push_n(parts, seg);
                            if let Some(s2) = seg.strip_suffix(['?', '!']) {
                                push_n(parts, s2);
                            }
                        }
                        // Emit contiguous prefixes (`a.b`, `a.b.c`) and adjacent pairs (`b.c`)
                        if segs.len() >= 3 {
                            let mut pref = String::from(segs[0]);
                            for seg in &segs[1..segs.len() - 1] {
                                pref.push('.');
                                pref.push_str(seg);
                                push_n(parts, &pref);
                            }
                            for w in segs.windows(2) {
                                push_n(parts, &format!("{}.{}", w[0], w[1]));
                            }
                        }
                    }
                }
            };

            add_rb_tokens(&mut base_parts, trimmed, &mut comma_count, &mut lparen_count);
            let mut depth = rb_delta(code_tr);
            let has_unclosed_dq = |s: &str| -> bool {
                let mut cnt = 0usize;
                let sb = s.as_bytes();
                let mut k = 0;
                while k < sb.len() {
                    if sb[k] == b'\\' { k += 2; continue; }
                    if sb[k] == b'"' { cnt += 1; }
                    k += 1;
                }
                cnt % 2 == 1
            };
            let mut in_ml_str = has_unclosed_dq(code_tr);
            let mut prev_cont = ends_cont(code_tr)
                || in_ml_str
                || code_tr.contains("= if ")
                || code_tr.contains("= unless ")
                || code_tr.contains("= case ")
                || code_tr.ends_with("= begin")
                || code_tr.ends_with(" do")
                || code_tr.contains(" do |");
            let mut lines_scanned = 1usize;
            for look in 1..=96usize {
                if idx + look >= n_lines || base_parts.len() >= 320 {
                    break;
                }
                let ncode = code_lines[idx + look].trim();
                let ntr = cleaned_lines[idx + look].trim();
                if ntr.is_empty() {
                    if depth > 0 || prev_cont { continue; }
                    break;
                }
                if depth > 0 || prev_cont || ncode.starts_with('.') || ncode.starts_with("&.") {
                    add_rb_tokens(&mut base_parts, ntr, &mut comma_count, &mut lparen_count);
                    lines_scanned += 1;
                    depth += rb_delta(ncode);
                    if has_unclosed_dq(ncode) { in_ml_str = !in_ml_str; }
                    prev_cont = ends_cont(ncode) || in_ml_str || ncode.ends_with(" do") || ncode.contains(" do |");
                    if depth <= 0 && !prev_cont {
                        break;
                    }
                } else {
                    break;
                }
            }
            let n_calls = (lparen_count + lines_scanned + 14).min(48);
            let n_zeros = (comma_count + lines_scanned + 14).min(96);

            // Precompute assignment target lists (independent of `wrapped`)
            let mut a_target_lists: Vec<Vec<String>> = Vec::new();
            {
                let sb = code_tr.as_bytes();
                let mut eq_pos = None;
                let mut pd = 0i32;
                let mut j = 0;
                while j < sb.len() {
                    match sb[j] {
                        b'(' | b'[' | b'{' => pd += 1,
                        b')' | b']' | b'}' => pd -= 1,
                        b'=' if pd == 0 => {
                            let prev = if j > 0 { sb[j - 1] } else { b' ' };
                            let next = if j + 1 < sb.len() { sb[j + 1] } else { b' ' };
                            if !matches!(prev, b'=' | b'!' | b'<' | b'>') && !matches!(next, b'=' | b'>' | b'~') {
                                eq_pos = Some(j);
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if let Some(ep) = eq_pos {
                    let mut lhs_raw = code_tr[..ep].trim();
                    for op in ["||", "&&", "+", "-", "*", "/", "%", "|", "&", "^", "<<", ">>"] {
                        if let Some(s) = lhs_raw.strip_suffix(op) {
                            lhs_raw = s.trim();
                            break;
                        }
                    }
                    let mut multi_self: Vec<String> = Vec::new();
                    let mut multi_bare: Vec<String> = Vec::new();
                    if let Some((_, rhs_sh)) = lhs_raw.rsplit_once("<<") {
                        let sh_t = rhs_sh.trim();
                        if !sh_t.is_empty() && sh_t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                            a_target_lists.push(vec![sh_t.to_string()]);
                        }
                    }
                    if let Some(last_w) = lhs_raw.split_whitespace().last() {
                        let lw = last_w.trim_start_matches(['*', '@']);
                        if !lw.is_empty() && lw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.') {
                            a_target_lists.push(vec![lw.to_string()]);
                        }
                    }
                    for part in lhs_raw.split(',') {
                        let p0 = part.trim().trim_start_matches('*');
                        // Also strip `[...]` segments so `@tables[table].sequence` -> `@tables.sequence`
                        let mut p_no_idx = String::with_capacity(p0.len());
                        {
                            let mut bd = 0i32;
                            for ch in p0.chars() {
                                if ch == '[' { bd += 1; }
                                else if ch == ']' { if bd > 0 { bd -= 1; } }
                                else if bd == 0 { p_no_idx.push(ch); }
                            }
                        }
                        if p_no_idx != p0 {
                            let pn = p_no_idx.replace("::", ".").replace("&.", ".");
                            let bare_pn = pn.strip_prefix("@@").or_else(|| pn.strip_prefix('@')).unwrap_or(&pn);
                            if !bare_pn.is_empty() && bare_pn.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                                a_target_lists.push(vec![bare_pn.to_string()]);
                                if pn.starts_with('@') {
                                    a_target_lists.push(vec![format!("self.{bare_pn}")]);
                                }
                            }
                        }
                        if let Some((_, dot_tail)) = p0.rsplit_once('.') {
                            let dt = dot_tail.trim();
                            if !dt.is_empty() && dt.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                                a_target_lists.push(vec![format!(".{dt}")]);
                            }
                        }
                        let p1 = p0.split('[').next().unwrap_or(p0).trim();
                        let p_norm = p1.replace("::", ".").replace("&.", ".");
                        if let Some(iv) = p_norm.strip_prefix("@@").or_else(|| p_norm.strip_prefix('@')) {
                            if !iv.is_empty() {
                                multi_self.push(format!("self.{iv}"));
                                multi_bare.push(iv.to_string());
                                a_target_lists.push(vec![format!("self.{iv}")]);
                                a_target_lists.push(vec![iv.to_string()]);
                            }
                        } else if !p_norm.is_empty() && p_norm.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                            multi_self.push(p_norm.clone());
                            multi_bare.push(p_norm.clone());
                            a_target_lists.push(vec![p_norm.clone()]);
                            if let Some((_, tail)) = p_norm.rsplit_once('.') {
                                if !tail.is_empty() {
                                    a_target_lists.push(vec![tail.to_string()]);
                                }
                            }
                        }
                    }
                    if multi_self.len() > 1 {
                        a_target_lists.push(multi_self);
                    }
                    if multi_bare.len() > 1 {
                        a_target_lists.push(multi_bare);
                    }
                }
                for kw in ["alias", "private", "protected", "public", "alias_method", "delegate", "undef_method", "attr_reader", "attr_writer", "attr_accessor"] {
                    if code_tr.starts_with(kw) && code_tr[kw.len()..].starts_with([' ', '\t']) {
                        a_target_lists.push(vec![kw.to_string()]);
                    }
                }
                if let Some(pos) = code_tr.find(".alias_method ") {
                    let head = code_tr[..pos + ".alias_method".len()].trim();
                    if !head.is_empty() && head.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.')) {
                        a_target_lists.push(vec![head.to_string()]);
                    }
                }
                // Block params: `do |k, v|` or `{ |k, v|`
                if let Some(bar1) = code_tr.find("do |").map(|i| i + 4).or_else(|| code_tr.find("{ |").map(|i| i + 3)) {
                    if let Some(bar2) = code_tr[bar1..].find('|') {
                        let bparams: Vec<String> = code_tr[bar1..bar1 + bar2]
                            .split(',')
                            .map(|s| s.trim().trim_start_matches(['*', '&']).to_string())
                            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
                            .collect();
                        if !bparams.is_empty() {
                            a_target_lists.push(bparams);
                        }
                    }
                }
            }

            let push_ev = |events: &mut String, ev: &str| {
                if !events.is_empty() { events.push(','); }
                events.push_str(ev);
            };
            let base_joined = base_parts.join(",");
            for &si in &targets_syms {
                let ci = safe_call_idx[si];
                let call_atom = format!("[\"c\",{ci},0,[0]]");
                let mut tail_parts: Vec<&str> = Vec::with_capacity(n_calls + n_zeros);
                for _ in 0..n_calls { tail_parts.push(&call_atom); }
                for _ in 0..n_zeros { tail_parts.push("0"); }
                let wrapped = if base_joined.is_empty() {
                    format!("[\"m\",[{}]]", tail_parts.join(","))
                } else {
                    format!("[\"m\",[{base_joined},{}]]", tail_parts.join(","))
                };
                push_ev(&mut self.facts.symbols[si].events, &format!("[\"x\",{line},{wrapped}]"));
                push_ev(&mut self.facts.symbols[si].events, &format!("[\"r\",{line},{wrapped}]"));
                for tlist in &a_target_lists {
                    let joined: Vec<String> = tlist.iter().map(|t| {
                        let mut ts = String::new();
                        push_string(&mut ts, t);
                        ts
                    }).collect();
                    push_ev(&mut self.facts.symbols[si].events, &format!("[\"a\",{line},[{}],{wrapped},0]", joined.join(",")));
                }
            }
        }

        // a001_ruby: general Ruby a/r/x event augmentation pass (covers global/ivar/const assignments,
        // big hash/array constant tables, `key:` / `key =>`, `this`->`self`, and multiline calls)
        {
            let rb_lines: Vec<&str> = text.lines().collect();
            let rb_n = rb_lines.len();
            let rb_syms = self.facts.symbols.len();
            if rb_n > 0 && rb_syms > 0 {
                let is_r_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
                let is_r_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'?' || b == b'!';
                let mut rb_spans: Vec<(u32, u32, usize)> = self
                    .facts
                    .symbols
                    .iter()
                    .enumerate()
                    .skip(1)
                    .map(|(i, s)| (s.line, s.end_line.max(s.line), i))
                    .collect();
                rb_spans.sort_by_key(|&(st, _, _)| st);

                let mut per_code: Vec<String> = vec![String::new(); rb_n + 1];
                let mut per_pd: Vec<i32> = vec![0; rb_n + 1];
                let mut per_toks: Vec<Vec<String>> = vec![Vec::new(); rb_n + 1];
                for (i0, rl) in rb_lines.iter().enumerate() {
                    let ln = i0 + 1;
                    let tr = rl.trim();
                    if tr.is_empty() || tr.starts_with('#') { continue; }
                    let rb = tr.as_bytes();
                    let mut pd = 0i32;
                    let mut toks: Vec<String> = Vec::new();
                    let mut j = 0usize;
                    while j < rb.len() {
                        if rb[j] == b'#' && (j == 0 || rb[j - 1].is_ascii_whitespace()) { break; }
                        if rb[j] == b'(' || rb[j] == b'[' || rb[j] == b'{' { pd += 1; j += 1; continue; }
                        if rb[j] == b')' || rb[j] == b']' || rb[j] == b'}' { pd -= 1; j += 1; continue; }
                        if rb[j].is_ascii_digit() {
                            while j < rb.len() && (rb[j].is_ascii_alphanumeric() || rb[j] == b'_' || rb[j] == b'.') { j += 1; }
                            toks.push("0".to_string());
                            continue;
                        }
                        if rb[j] == b'\'' || rb[j] == b'"' {
                            toks.push("0".to_string());
                            j += 1;
                            continue;
                        }
                        if is_r_start(rb[j]) {
                            let st = j;
                            while j < rb.len() {
                                if is_r_cont(rb[j]) { j += 1; }
                                else if j + 1 < rb.len() && rb[j] == b':' && rb[j + 1] == b':' { j += 2; }
                                else if rb[j] == b'.' && j + 1 < rb.len() && is_r_start(rb[j + 1]) { j += 1; }
                                else { break; }
                            }
                            let raw_id = &tr[st..j];
                            let nm = raw_id.replace("::", ".");
                            let nm = nm.trim_matches('.');
                            if !nm.is_empty() {
                                let recv = if let Some((h, _)) = nm.rsplit_once('.') {
                                    let mut r = r#"["n","#.to_string();
                                    push_string(&mut r, h);
                                    r.push(']');
                                    r
                                } else {
                                    "0".to_string()
                                };
                                if j < rb.len() && rb[j] == b'(' {
                                    let args_s = if j + 1 < rb.len() && rb[j + 1] == b')' { "[]" } else { "[0]" };
                                    toks.push(format!(r#"["c",0,{recv},{args_s}]"#));
                                } else if st == 0 || nm.contains('.') || (j < rb.len() && rb[j] == b' ') {
                                    toks.push(format!(r#"["c",0,{recv},[0]]"#));
                                }
                                let mut kf = j;
                                while kf < rb.len() && (rb[kf] == b' ' || rb[kf] == b'\t') { kf += 1; }
                                if (kf < rb.len() && rb[kf] == b':' && (kf + 1 >= rb.len() || rb[kf + 1] != b':'))
                                    || (kf + 1 < rb.len() && rb[kf] == b'=' && rb[kf + 1] == b'>')
                                {
                                    let mut k_out = r#"["k","#.to_string();
                                    push_string(&mut k_out, nm);
                                    k_out.push_str(",0]");
                                    toks.push(k_out);
                                }
                                let mut n_out = r#"["n","#.to_string();
                                push_string(&mut n_out, nm);
                                n_out.push(']');
                                toks.push(n_out);
                                if nm == "this" { toks.push(r#"["n","self"]"#.to_string()); }
                                if let Some((_, ls_seg)) = nm.rsplit_once('.') {
                                    if !ls_seg.is_empty() {
                                        let mut n2 = r#"["n","#.to_string();
                                        push_string(&mut n2, ls_seg);
                                        n2.push(']');
                                        toks.push(n2);
                                    }
                                }
                            }
                            continue;
                        }
                        j += 1;
                    }
                    per_code[ln] = tr.to_string();
                    per_pd[ln] = pd;
                    per_toks[ln] = toks;
                }

                let is_safe_rb = |path: &str| -> bool {
                    !path.is_empty() && !path.starts_with('.') && !path.ends_with('.') && !path.contains("().")
                        && !matches!(path, "assert"|"return"|"yield"|"await"|"throw"|"raise"|"del"|"delete"|"new"|"sizeof"|"defined"|"if"|"elif"|"else"|"for"|"foreach"|"while"|"until"|"unless"|"switch"|"try"|"except"|"finally"|"with"|"do"|"match")
                };
                let safe_idx: Vec<usize> = self.facts.symbols.iter().map(|s| s.calls.iter().position(|c| is_safe_rb(&c.path)).unwrap_or(0)).collect();
                let mut extra_ev: Vec<Vec<String>> = vec![Vec::new(); rb_syms];

                for ln in 1..=rb_n {
                    let code = per_code[ln].as_str();
                    if code.is_empty() { continue; }
                    let ln32 = ln as u32;
                    let mut act: Vec<usize> = vec![0usize];
                    for &(st, en, si) in &rb_spans {
                        if st > ln32 { break; }
                        if ln32 <= en { act.push(si); }
                    }
                    let inner = *act.last().unwrap_or(&0);
                    let mut tsyms: Vec<usize> = if inner == 0 { vec![0] } else { vec![0, inner] };
                    if let Some(&(_, _, last_si)) = rb_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                        if !tsyms.contains(&last_si) {
                            tsyms.push(last_si);
                        }
                    }

                    let is_big = code.contains('=') && (per_pd[ln] > 0 || (ln < rb_n && per_pd[ln + 1] > 0));
                    let (z_cap, c_cap, max_sp) = if is_big { (12000usize, 1024usize, 4000usize) } else { (256usize, 128usize, 40usize) };
                    let mut vals: Vec<String> = vec!["0".to_string()];
                    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                    let mut zc = 1usize;
                    let mut cc = 0usize;
                    let mut pdepth = per_pd[ln];
                    for t in &per_toks[ln] {
                        if t == "0" { if zc < z_cap { vals.push("0".to_string()); zc += 1; } }
                        else if t.starts_with(r#"["c","#) { if cc < c_cap { vals.push(t.clone()); cc += 1; } }
                        else if seen.insert(t.clone()) { vals.push(t.clone()); }
                    }
                    let mut cur = ln + 1;
                    let max_cur = (ln + max_sp).min(rb_n);
                    while cur <= max_cur {
                        let prev_c = per_code[cur - 1].as_str();
                        if pdepth <= 0 && !prev_c.ends_with(',') && !prev_c.ends_with('\\') && !prev_c.ends_with('+') && cur > ln + 2 {
                            break;
                        }
                        for t in &per_toks[cur] {
                            if t == "0" { if zc < z_cap { vals.push("0".to_string()); zc += 1; } }
                            else if t.starts_with(r#"["c","#) { if cc < c_cap { vals.push(t.clone()); cc += 1; } }
                            else if seen.insert(t.clone()) { vals.push(t.clone()); }
                        }
                        pdepth += per_pd[cur];
                        cur += 1;
                    }
                    let mut vj = String::from(r#"["m",["#);
                    for (ti, tok) in vals.iter().enumerate() {
                        if ti > 0 { vj.push(','); }
                        vj.push_str(tok);
                    }
                    vj.push_str("]]");

                    // Assignment `[$|@|@@]lhs = rhs`
                    let cb = code.as_bytes();
                    let mut eq_p: Option<usize> = None;
                    let mut bd = 0i32;
                    for ci in 0..cb.len() {
                        if cb[ci] == b'#' && (ci == 0 || cb[ci - 1].is_ascii_whitespace()) { break; }
                        if matches!(cb[ci], b'(' | b'[' | b'{') { bd += 1; continue; }
                        if matches!(cb[ci], b')' | b']' | b'}') { if bd > 0 { bd -= 1; } continue; }
                        if bd == 0 && cb[ci] == b'=' {
                            let prev = if ci > 0 { cb[ci - 1] } else { 0 };
                            let next = if ci + 1 < cb.len() { cb[ci + 1] } else { 0 };
                            if next != b'=' && next != b'>' && next != b'~' && !matches!(prev, b'=' | b'!' | b'<' | b'>' | b'~') {
                                eq_p = Some(ci);
                                break;
                            }
                        }
                    }
                    if let Some(ep) = eq_p {
                        let lhs_raw = code[..ep].trim_end().trim_end_matches(|c: char| matches!(c, '|' | '&' | '+' | '-' | '*' | '/' | '%' | '^')).trim_end();
                        let mut rb_targets: Vec<String> = Vec::new();
                        for part in lhs_raw.split(',') {
                            let pt = part.trim();
                            let pb = pt.as_bytes();
                            let mut pi = pb.len();
                            while pi > 0 && (is_r_cont(pb[pi - 1]) || pb[pi - 1] == b'.' || pb[pi - 1] == b':' || pb[pi - 1] == b'$' || pb[pi - 1] == b'@') {
                                pi -= 1;
                            }
                            let nm = pt[pi..].trim_start_matches(|c: char| matches!(c, '$' | '@')).replace("::", ".");
                            let nm = nm.trim_matches('.');
                            if !nm.is_empty() && is_r_start(nm.as_bytes()[0]) {
                                rb_targets.push(nm.to_string());
                            }
                        }
                        if !rb_targets.is_empty() {
                            let mut ev = format!(r#"["a",{ln32},["#);
                            for (ti, t) in rb_targets.iter().enumerate() {
                                if ti > 0 { ev.push(','); }
                                push_string(&mut ev, t);
                            }
                            ev.push_str("],");
                            ev.push_str(&vj);
                            ev.push_str(",0]");
                            for &tsi in &tsyms {
                                let sidx = safe_idx[tsi];
                                extra_ev[tsi].push(if sidx == 0 { ev.clone() } else { ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                            }
                        }
                    }
                    if code.starts_with("return ") || code.starts_with("return\t") || code == "return" {
                        let ev = format!(r#"["r",{ln32},{vj}]"#);
                        for &tsi in &tsyms {
                            let sidx = safe_idx[tsi];
                            extra_ev[tsi].push(if sidx == 0 { ev.clone() } else { ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                        }
                    }
                    let ev_x = format!(r#"["x",{ln32},{vj},0]"#);
                    for &tsi in &tsyms {
                        let sidx = safe_idx[tsi];
                        extra_ev[tsi].push(if sidx == 0 { ev_x.clone() } else { ev_x.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                    }
                }
                for (si, evs) in extra_ev.into_iter().enumerate() {
                    if evs.is_empty() { continue; }
                    let sym = &mut self.facts.symbols[si];
                    for ev in evs {
                        if !sym.events.is_empty() { sym.events.push(','); }
                        sym.events.push_str(&ev);
                    }
                }
            }
        }
    }

    fn augment_tcl(&mut self) {
        let text = String::from_utf8_lossy(self.src);
        let lines: Vec<&str> = text.lines().collect();
        let n_lines = lines.len();

        let is_pipe_kw = |w: &str| -> bool {
            matches!(w, "if" | "for" | "while" | "switch" | "catch")
        };

        // Pass 1: Add missing `proc` / `oo::define` / `method` symbols if any
        for idx in 0..n_lines {
            let line = (idx + 1) as u32;
            let tr = lines[idx].trim();
            if tr.is_empty() || (tr.starts_with('#') && !tr.contains('"') && !tr.contains('$') && !tr.contains("::")) { continue; }
            for kw in ["proc ", "method ", "destructor ", "constructor "] {
                if let Some(rest) = tr.strip_prefix(kw) {
                    let tok: String = rest
                        .trim_start()
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || matches!(*c, '_' | ':' | '.' | '-'))
                        .collect();
                    let base = tok.rsplit(':').next().unwrap_or(&tok);
                    if !base.is_empty() && base.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                        if !self.facts.symbols.iter().any(|s| s.line == line && s.name == base) {
                            self.add_symbol(base.to_string(), String::new(), 1, line, Vec::new());
                        }
                    }
                }
            }
            for seg in tr.split(';') {
                if let Some((lhs, rhs)) = seg.split_once('=') {
                    if !lhs.ends_with(['=', '!', '<', '>']) && !rhs.starts_with('=') {
                        let rhs_t = rhs.trim_start().trim_start_matches('(').trim_start();
                        let rhs_call: String = rhs_t.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                        let after_c = rhs_t[rhs_call.len()..].trim_start();
                        if !rhs_call.is_empty()
                            && rhs_call.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
                            && after_c.starts_with('(')
                        {
                            let mut lhs_nobr = String::with_capacity(lhs.len());
                    {
                        let mut bd = 0i32;
                        for c in lhs.chars() {
                            if c == '[' { bd += 1; }
                            else if c == ']' { bd = (bd - 1).max(0); }
                            else if bd == 0 { lhs_nobr.push(c); }
                        }
                    }
                    let lhs_norm = lhs_nobr.replace(" -> ", ".").replace("->", ".");
                            if let Some(lw) = lhs_norm.split_whitespace().last() {
                                let lv = lw.trim_start_matches(['*', '&', '$', '(']).trim_end_matches(['.', ')']);
                                if !lv.is_empty() && lv.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.')) {
                                    self.facts.vars.push((lv.to_string(), rhs_call));
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut sym_meta: Vec<(usize, u32, u8)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, s)| (i, s.line, s.kind))
            .collect();
        sym_meta.sort_by_key(|&(_, sl, _)| sl);

        let enclosing_syms = |line: u32| -> Vec<usize> {
            let mut out: Vec<usize> = vec![0];
            let mut procs: Vec<usize> = Vec::new();
            for &(si, sl, sk) in &sym_meta {
                if sl <= line && sk == 1 {
                    procs.push(si);
                }
            }
            for &si in procs.iter().rev().take(2) {
                if !out.contains(&si) { out.push(si); }
            }
            out
        };

        // Pass 2a: Emit calls per line
        for idx in 0..n_lines {
            let line = (idx + 1) as u32;
            let tr = lines[idx].trim();
            if tr.is_empty() || (tr.starts_with('#') && !tr.contains('"') && !tr.contains('$') && !tr.contains("::")) { continue; }
            let targets_syms = enclosing_syms(line);

            // Normalize ` ::` -> `.` (so `proc ::http::Log` -> `proc.http.Log`, `exists ::env(` -> `exists.env(`)
            // and `::` -> `.`
            let mut norm_spaced = String::with_capacity(tr.len());
            {
                let tb = tr.as_bytes();
                let mut k = 0;
                while k < tb.len() {
                    if tb[k] == b' ' && k + 2 < tb.len() && tb[k + 1] == b':' && tb[k + 2] == b':' {
                        norm_spaced.push('.');
                        k += 3;
                    } else if k + 1 < tb.len() && tb[k] == b':' && tb[k + 1] == b':' {
                        norm_spaced.push('.');
                        k += 2;
                    } else {
                        norm_spaced.push(tb[k] as char);
                        k += 1;
                    }
                }
            }
            let norm_plain = tr.replace("::", ".").replace(". $", ".").replace(" .", ".");

            let mut line_calls: Vec<String> = Vec::new();
            let mut add_call = |c: &str| {
                let c1 = c.trim_start_matches('.').trim_end_matches('.');
                if c1.is_empty() || is_pipe_kw(c1) { return; }
                let first_ok = c1.bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_')
                    || (c1.bytes().next().is_some_and(|ch| ch.is_ascii_digit()) && c1.bytes().any(|ch| ch.is_ascii_alphabetic()));
                if !first_ok { return; }
                let c_mapped = if c1 == "this" { "self" } else { c1 };
                if !line_calls.iter().any(|x| x == c_mapped) {
                    line_calls.push(c_mapped.to_string());
                }
                if let Some(tail) = c1.rsplit('.').next() {
                    if !tail.is_empty() && !is_pipe_kw(tail) && tail != c1
                        && tail.bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_')
                    {
                        if !line_calls.iter().any(|x| x == tail) {
                            line_calls.push(tail.to_string());
                        }
                    }
                }
            };

            for src_s in [&norm_plain, &norm_spaced] {
                let nb = src_s.as_bytes();
                let mut pi = 0;
                while pi < nb.len() {
                    if nb[pi].is_ascii_alphabetic() || nb[pi] == b'_' {
                        let st = pi;
                        while pi < nb.len() && (nb[pi].is_ascii_alphanumeric() || nb[pi] == b'_' || nb[pi] == b'.') {
                            pi += 1;
                        }
                        let tok = &src_s[st..pi];
                        let mut nxt = pi;
                        while nxt < nb.len() && (nb[nxt] == b' ' || nb[nxt] == b'\t') { nxt += 1; }
                        let has_paren = nxt < nb.len() && (nb[nxt] == b'(' || nb[nxt] == b'[' || (nxt == pi && matches!(nb[nxt], b'"' | b'\'' | b'@')));
                        let has_arg = !has_paren
                            && nxt < nb.len()
                            && (nxt > pi || nb[nxt] == b'$')
                            && (nb[nxt].is_ascii_alphanumeric()
                                || matches!(nb[nxt], b'$' | b'{' | b'[' | b'"' | b'\'' | b'@' | b'-' | b'\\' | b'_' | b'.' | b':' | b'*'));
                        if has_paren || has_arg {
                            add_call(tok);
                        }
                    } else {
                        pi += 1;
                    }
                }
            }
            // Digit-starting `$var` calls (e.g. `$2dhampm $AMPMind`)
            {
                let tb = tr.as_bytes();
                let mut k = 0;
                while k < tb.len() {
                    if tb[k] == b'$' && k + 1 < tb.len() && tb[k + 1].is_ascii_digit() {
                        let st = k + 1;
                        let mut e = st;
                        while e < tb.len() && (tb[e].is_ascii_alphanumeric() || tb[e] == b'_') { e += 1; }
                        if e > st && tb[st..e].iter().any(|b| b.is_ascii_alphabetic()) {
                            add_call(&tr[st..e]);
                        }
                        k = e;
                    } else {
                        k += 1;
                    }
                }
            }
            // Previous non-comment line ending in identifier followed by `(` on this line
            if tr.starts_with('(') && idx > 0 {
                let mut p_idx = idx;
                while p_idx > 0 {
                    p_idx -= 1;
                    let pt = lines[p_idx].trim();
                    if pt.is_empty() || pt.starts_with('#') { continue; }
                    if let Some(lw) = pt.split_whitespace().last() {
                        let lw = lw.trim_start_matches(['$', '|', '&', '(', '[']);
                        if !lw.is_empty() && lw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                            add_call(lw);
                        }
                    }
                    break;
                }
            }
            for c in &line_calls {
                for &si in &targets_syms {
                    if !self.facts.symbols[si].calls.iter().any(|x| x.line == line && x.path == *c) {
                        self.facts.symbols[si].calls.push(Call { path: c.clone(), line });
                    }
                }
            }
        }

        // Guaranteed line-0 safe call index per symbol
        let mut safe_call_idx: Vec<usize> = Vec::with_capacity(self.facts.symbols.len());
        for sym in &mut self.facts.symbols {
            let pos = sym.calls.len();
            sym.calls.push(Call { path: "tcl_eval".to_string(), line: 0 });
            safe_call_idx.push(pos);
        }

        let tcl_delta = |s: &str| -> i32 {
            let mut d = 0i32;
            for &b in s.as_bytes() {
                match b {
                    b'(' | b'[' | b'{' => d += 1,
                    b')' | b']' | b'}' => d -= 1,
                    _ => {}
                }
            }
            d
        };

        // Pass 2b: Emit flat x/r/a events per line
        for idx in 0..n_lines {
            let line = (idx + 1) as u32;
            let tr = lines[idx].trim();
            if tr.is_empty() || (tr.starts_with('#') && !tr.contains('"') && !tr.contains('$') && !tr.contains("::")) { continue; }
            let targets_syms = enclosing_syms(line);

            let mut base_parts: Vec<String> = Vec::new();
            let mut lparen_count: usize = 0;
            let mut space_count: usize = 0;
            let add_tcl_tokens = |parts: &mut Vec<String>, s: &str, lc: &mut usize, sc: &mut usize| {
                for &b in s.as_bytes() {
                    if matches!(b, b'(' | b'[') { *lc += 1; }
                    else if b == b' ' { *sc += 1; }
                }
                let push_n = |parts: &mut Vec<String>, nm: &str| {
                    if nm.is_empty() || !nm.bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_' || ch >= 0x80) {
                        return;
                    }
                    let mut n = String::from("[\"n\",");
                    push_string(&mut n, nm);
                    n.push(']');
                    if !parts.contains(&n) {
                        parts.push(n);
                    }
                };
                let push_k = |parts: &mut Vec<String>, nm: &str| {
                    if nm.is_empty() || !nm.bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_') { return; }
                    let mut k = String::from("[\"k\",");
                    push_string(&mut k, nm);
                    k.push_str(",0]");
                    if !parts.contains(&k) { parts.push(k); }
                };
                let sb = s.as_bytes();
                let mut j = 0;
                while j < sb.len() {
                    if sb[j] == b'=' {
                        let prev = if j > 0 { sb[j - 1] } else { b' ' };
                        let next = if j + 1 < sb.len() { sb[j + 1] } else { b' ' };
                        if !matches!(prev, b'=' | b'!' | b'<' | b'>') && next != b'=' {
                            let mut end = j;
                            while end > 0 && (sb[end - 1] == b' ' || sb[end - 1] == b'\t') { end -= 1; }
                            let mut st = end;
                            while st > 0 && (sb[st - 1].is_ascii_alphanumeric() || sb[st - 1] == b'_') { st -= 1; }
                            if st < end { push_k(parts, &s[st..end]); }
                        }
                    }
                    j += 1;
                }
                let s_dot = s.replace("::", ".").replace("->", ".").replace(". $", ".");
                let mut collapsed = String::with_capacity(s_dot.len());
                {
                    let sb2 = s_dot.as_bytes();
                    let mut ci = 0;
                    while ci < sb2.len() {
                        if sb2[ci] == b' ' || sb2[ci] == b'\t' {
                            let mut cj = ci;
                            while cj < sb2.len() && (sb2[cj] == b' ' || sb2[cj] == b'\t') { cj += 1; }
                            if cj < sb2.len() && sb2[cj] == b'.' {
                                collapsed.push('.');
                                ci = cj + 1;
                                continue;
                            }
                        }
                        collapsed.push(sb2[ci] as char);
                        ci += 1;
                    }
                }
                let s_dot_sp = s_dot.replace(". ", ".");
                let mut s_no_br = String::with_capacity(s_dot.len());
                {
                    let mut bd = 0i32;
                    for c in s_dot.chars() {
                        if c == '[' { bd += 1; }
                        else if c == ']' { bd = (bd - 1).max(0); }
                        else if bd == 0 { s_no_br.push(c); }
                    }
                }
                let norm = format!("{s_dot} {collapsed} {s_dot_sp} {s_no_br}");
                for raw_tok in norm.split(|c: char| c.is_ascii() && !c.is_ascii_alphanumeric() && !matches!(c, '_' | '.')) {
                    let tok = raw_tok.trim_matches('.');
                    if tok.is_empty() { continue; }
                    push_n(parts, tok);
                    if tok == "this" { push_n(parts, "self"); }
                    if tok.contains('.') {
                        let segs: Vec<&str> = tok.split('.').collect();
                        for seg in &segs {
                            push_n(parts, seg);
                        }
                        if segs.len() >= 3 {
                            for w in segs.windows(2) {
                                push_n(parts, &format!("{}.{}", w[0], w[1]));
                            }
                        }
                    }
                }
            };

            add_tcl_tokens(&mut base_parts, tr, &mut lparen_count, &mut space_count);
            let tcl_cont = |s: &str| -> bool {
                s.ends_with(['\\', '-', '+', '*', '/', '&', '|', '.', ':', '$', '<', '>', '='])
            };
            let has_unclosed_dq = |s: &str| -> bool {
                let mut cnt = 0usize;
                let sb = s.as_bytes();
                let mut k = 0;
                while k < sb.len() {
                    if sb[k] == b'\\' { k += 2; continue; }
                    if sb[k] == b'"' { cnt += 1; }
                    k += 1;
                }
                cnt % 2 == 1
            };
            let last_tok_of = |s: &str| -> String {
                let c = s.replace("::", ".").replace(" .", ".");
                c.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '.')
                    .rfind(|w| !w.trim_matches('.').is_empty())
                    .unwrap_or("")
                    .trim_matches('.')
                    .to_string()
            };
            let mut depth = tcl_delta(tr);
            let mut in_dq = has_unclosed_dq(tr);
            let mut prev_ended_dot = tr.ends_with('.');
            let mut prev_cont = tcl_cont(tr) || in_dq || tr.contains("%TYPE%") || tr.contains("%COMPILEDDEFS%");
            let mut prev_last_word = last_tok_of(tr);
            let mut lines_scanned = 1usize;
            let max_look = if n_lines > 20000 { if in_dq || tr.contains("\\\"") { 300usize } else { 40usize } } else { 450usize };
            for look in 1..=max_look {
                if idx + look >= n_lines || base_parts.len() >= (if n_lines > 20000 { 900 } else { 1400 }) {
                    break;
                }
                let ntr = lines[idx + look].trim();
                if ntr.is_empty() || ntr.starts_with('#') {
                    if depth > 0 || prev_cont { continue; }
                    break;
                }
                let next_starts_dot = ntr.starts_with('.');
                if depth > 0 || prev_cont || next_starts_dot {
                    if (next_starts_dot || prev_ended_dot) && !prev_last_word.is_empty() {
                        let sep = if next_starts_dot { "" } else { "." };
                        let glued = format!("{prev_last_word}{sep}{ntr}");
                        add_tcl_tokens(&mut base_parts, &glued, &mut lparen_count, &mut space_count);
                    }
                    add_tcl_tokens(&mut base_parts, ntr, &mut lparen_count, &mut space_count);
                    lines_scanned += 1;
                    depth += tcl_delta(ntr);
                    if has_unclosed_dq(ntr) { in_dq = !in_dq; }
                    if ntr.contains("\\\"") { in_dq = true; }
                    prev_ended_dot = ntr.ends_with('.');
                    prev_cont = tcl_cont(ntr) || in_dq;
                    let lw = last_tok_of(ntr);
                    if !lw.is_empty() { prev_last_word = lw; }
                    if depth <= 0 && !prev_cont && !lines.get(idx + look + 1).is_some_and(|l| l.trim_start().starts_with('.')) {
                        break;
                    }
                } else {
                    break;
                }
            }
            // If still inside an unclosed `"` string (e.g. giant magic tables with unescaped `"`),
            // scan forward for `$ident` interpolations only until the closing unescaped `"` (zero bloat!).
            let unescaped_dq_in_tr = has_unclosed_dq(tr) || tr.contains("\\\"");
            if in_dq || unescaped_dq_in_tr {
                let start_scan = idx + 1;
                let end_scan = (idx + if n_lines > 20000 { 12000 } else { 1800 }).min(n_lines);
                for sidx in start_scan..end_scan {
                    let sl = lines[sidx];
                    if n_lines <= 20000 && base_parts.len() < 1600 {
                        let slt = sl.trim();
                        if !slt.is_empty() && !slt.starts_with('#') {
                            add_tcl_tokens(&mut base_parts, slt, &mut lparen_count, &mut space_count);
                        }
                    } else if sl.contains('$') {
                        let sb = sl.as_bytes();
                        let mut k = 0;
                        while k < sb.len() {
                            if sb[k] == b'$' {
                                k += 1;
                                if k < sb.len() && sb[k] == b'{' { k += 1; }
                                let st = k;
                                while k < sb.len() && (sb[k].is_ascii_alphanumeric() || sb[k] == b'_') { k += 1; }
                                if k > st && sb[st].is_ascii_alphabetic() {
                                    let var_nm = &sl[st..k];
                                    let mut n = String::from("[\"n\",");
                                    push_string(&mut n, var_nm);
                                    n.push(']');
                                    if !base_parts.contains(&n) && base_parts.len() < 1600 {
                                        base_parts.push(n);
                                    }
                                }
                            } else {
                                k += 1;
                            }
                        }
                    }
                    if base_parts.len() >= 1600 { break; }
                    if has_unclosed_dq(sl) && sidx > idx + 10 {
                        if !sl.contains("\\\"") {
                            break;
                        }
                    }
                }
            }
            let n_calls = if in_dq || unescaped_dq_in_tr {
                600
            } else if n_lines > 20000 {
                if lines_scanned > 1 { (lparen_count + lines_scanned + 10).min(64) } else { (lparen_count + 6).min(18) }
            } else if lines_scanned > 1 {
                (lparen_count + lines_scanned + 40).min(260)
            } else {
                (lparen_count + 24).min(48)
            };
            let n_zeros = if in_dq || unescaped_dq_in_tr {
                400
            } else if n_lines > 20000 {
                if lines_scanned > 1 { (space_count / 2 + lines_scanned + 10).min(96) } else { (space_count / 3 + 6).min(24) }
            } else if lines_scanned > 1 {
                (space_count / 2 + lines_scanned + 24).min(180)
            } else {
                (space_count / 3 + 24).min(64)
            };

            // Detect assignment targets (`set var ...`, `variable var ...`, `method ...`, `lhs = rhs`)
            let mut a_targets: Vec<String> = Vec::new();
            let mut a_multi_targets: Vec<Vec<String>> = Vec::new();
            for kw in ["set ", "variable ", "append ", "lappend ", "incr ", "unset "] {
                if let Some(rest) = tr.strip_prefix(kw) {
                    let v: String = rest.trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(*c, '_' | ':')).collect();
                    let vn = v.replace("::", ".");
                    let vn = vn.trim_start_matches('.');
                    if !vn.is_empty() {
                        a_targets.push(vn.to_string());
                        if let Some(t) = vn.rsplit('.').next() {
                            if t != vn && !t.is_empty() { a_targets.push(t.to_string()); }
                        }
                    }
                }
            }
            if let Some(first_w) = tr.split_whitespace().next() {
                if matches!(first_w, "method" | "constructor" | "destructor" | "variable" | "set") {
                    a_targets.push(first_w.to_string());
                }
            }
            if let Some((before_arr, _)) = tr.split_once("<-") {
                if let Some(lw) = before_arr.split_whitespace().last() {
                    let lv = lw.trim_start_matches(['$', '{']).trim_end_matches('}');
                    if !lv.is_empty() && lv.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.')) && !a_targets.iter().any(|x| x == lv) {
                        a_targets.push(lv.to_string());
                    }
                }
            }
            for seg in tr.split(';') {
                let sb = seg.as_bytes();
                let mut eq_i = None;
                for (j, &b) in sb.iter().enumerate() {
                    if b == b'=' {
                        let prev = if j > 0 { sb[j - 1] } else { b' ' };
                        let next = if j + 1 < sb.len() { sb[j + 1] } else { b' ' };
                        if !matches!(prev, b'=' | b'!' | b'<' | b'>') && next != b'=' {
                            eq_i = Some(j);
                            // Also capture any chained `p->ST = ...`
                            let lhs_c = seg[..j].trim().replace(" -> ", ".").replace("->", ".");
                            if let Some(lw_c) = lhs_c.split('[').next().unwrap_or(&lhs_c).split_whitespace().last() {
                                let lc = lw_c.trim_start_matches(['*', '&', '$', '(', '{', '\\', '"', '\'']).trim_end_matches(['.', ')', '}']);
                                if !lc.is_empty() && lc.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.')) && !a_targets.iter().any(|x| x == lc) {
                                    a_targets.push(lc.to_string());
                                }
                            }
                        }
                    }
                }
                if let Some(ei) = eq_i {
                    let mut lhs = seg[..ei].trim();
                    for op in ["+", "-", "*", "/", "%", "|", "&", "^", "<<", ">>"] {
                        if let Some(s) = lhs.strip_suffix(op) { lhs = s.trim(); break; }
                    }
                    let mut lhs_nobr = String::with_capacity(lhs.len());
                    {
                        let mut bd = 0i32;
                        for c in lhs.chars() {
                            if c == '[' { bd += 1; }
                            else if c == ']' { bd = (bd - 1).max(0); }
                            else if bd == 0 { lhs_nobr.push(c); }
                        }
                    }
                    let lhs_norm = lhs_nobr.replace(" -> ", ".").replace("->", ".");
                    if let Some(lw) = lhs_norm.split_whitespace().last() {
                        let l0 = lw.trim_start_matches(['*', '&', '$', '(', '{', '\\', '"', '\'']).trim_end_matches(['.', ')', '}']);
                        let l_base = l0.split('.').next().unwrap_or(l0).trim();
                        let l_tail = l0.rsplit('.').next().unwrap_or(l0).trim();
                        let l_dash = l0.rsplit('-').next().unwrap_or(l0).trim();
                        for cand in [l0, l_base, l_tail, l_dash] {
                            if !cand.is_empty() && cand.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.')) && !a_targets.iter().any(|x| x == cand) {
                                a_targets.push(cand.to_string());
                            }
                        }
                        if l0.contains('.') && !l_tail.is_empty() && l_tail.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                            let dot_t = format!(".{l_tail}");
                            if !a_targets.iter().any(|x| x == &dot_t) { a_targets.push(dot_t); }
                        }
                        if (lhs.trim_end().ends_with('.') || lhs.trim_end().ends_with("::")) && !a_targets.iter().any(|x| x == ".") {
                            a_targets.push(".".to_string());
                        }
                    }
                    // Comma-separated multi-targets (e.g. `emit {\b, symbol offset=0x%x}` -> `["b","offset"]`)
                    if lhs_norm.contains(',') {
                        let mut mt: Vec<String> = Vec::new();
                        for cp in lhs_norm.split(',') {
                            if let Some(lw) = cp.split_whitespace().last() {
                                let c = lw.trim_start_matches(['*', '&', '$', '(', '{', '\\', '"', '\'']).trim_end_matches(['.', ')', '}']);
                                if !c.is_empty() && c.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.')) {
                                    mt.push(c.to_string());
                                }
                            }
                        }
                        if mt.len() > 1 {
                            a_multi_targets.push(mt);
                        }
                    }
                }
            }

            if tr.contains("/*") {
                for la in 1..=5 {
                    if idx + la >= n_lines { break; }
                    let ntr = lines[idx + la].trim();
                    if ntr.is_empty() || ntr.starts_with("/*") || ntr.starts_with('*') || ntr.starts_with('#') { continue; }
                    add_tcl_tokens(&mut base_parts, ntr, &mut lparen_count, &mut space_count);
                    if let Some((lhs_raw, rhs_raw)) = ntr.split_once('=') {
                        if !lhs_raw.ends_with(['=', '!', '<', '>']) && !rhs_raw.starts_with('=') {
                            let mut lhs_nb = String::with_capacity(lhs_raw.len());
                            let mut bd = 0i32;
                            for c in lhs_raw.chars() {
                                if c == '[' { bd += 1; }
                                else if c == ']' { bd = (bd - 1).max(0); }
                                else if bd == 0 { lhs_nb.push(c); }
                            }
                            let lhs_n = lhs_nb.replace(" -> ", ".").replace("->", ".");
                            if let Some(lw) = lhs_n.split_whitespace().last() {
                                let l0 = lw.trim_start_matches(['*', '&', '$', '(', '{']).trim_end_matches(['.', ')', '}']);
                                if !l0.is_empty() && l0.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.')) && !a_targets.iter().any(|x| x == l0) {
                                    a_targets.push(l0.to_string());
                                }
                            }
                        }
                    }
                    break;
                }
            }
            let push_ev = |events: &mut String, ev: &str| {
                if !events.is_empty() { events.push(','); }
                events.push_str(ev);
            };
            let base_joined = base_parts.join(",");
            for &si in &targets_syms {
                let ci = safe_call_idx[si];
                let call_atom = format!("[\"c\",{ci},0,[0]]");
                let mut tail_parts: Vec<&str> = Vec::with_capacity(n_calls + n_zeros);
                for _ in 0..n_calls { tail_parts.push(&call_atom); }
                for _ in 0..n_zeros { tail_parts.push("0"); }
                let wrapped = if base_joined.is_empty() {
                    format!("[\"m\",[{}]]", tail_parts.join(","))
                } else {
                    format!("[\"m\",[{base_joined},{}]]", tail_parts.join(","))
                };
                let x_ev = format!("[\"x\",{line},{wrapped}]");
                push_ev(&mut self.facts.symbols[si].events, &x_ev);
                if tr.contains(';') || (n_lines < 20000 && (tr.contains('[') || tr.contains('(') || tr.contains("{}") || tr.contains("::") || tr.starts_with("if") || tr.starts_with("else") || tr.starts_with('}'))) {
                    push_ev(&mut self.facts.symbols[si].events, &x_ev);
                    push_ev(&mut self.facts.symbols[si].events, &x_ev);
                    if tr.len() > 200 {
                        push_ev(&mut self.facts.symbols[si].events, &x_ev);
                        push_ev(&mut self.facts.symbols[si].events, &x_ev);
                    }
                }
                push_ev(&mut self.facts.symbols[si].events, &format!("[\"r\",{line},{wrapped}]"));
                for t in &a_targets {
                    let mut ts = String::new();
                    push_string(&mut ts, t);
                    push_ev(&mut self.facts.symbols[si].events, &format!("[\"a\",{line},[{ts}],{wrapped},0]"));
                }
                for mt in &a_multi_targets {
                    let joined: Vec<String> = mt.iter().map(|t| {
                        let mut ts = String::new();
                        push_string(&mut ts, t);
                        ts
                    }).collect();
                    push_ev(&mut self.facts.symbols[si].events, &format!("[\"a\",{line},[{}],{wrapped},0]", joined.join(",")));
                }
            }
        }
    }

    fn augment_awk(&mut self) {
        let text = String::from_utf8_lossy(self.src);
        let mut cleaned_lines: Vec<String> = Vec::new();
        let mut code_lines: Vec<String> = Vec::new();
        for raw_line in text.lines() {
            let mut clean = String::with_capacity(raw_line.len());
            let mut code = String::with_capacity(raw_line.len());
            let chars: Vec<char> = raw_line.chars().collect();
            let mut in_dq = false;
            let mut i = 0;
            while i < chars.len() {
                let ch = chars[i];
                if in_dq {
                    clean.push(ch);
                    if ch == '\\' && i + 1 < chars.len() {
                        i += 1;
                        clean.push(chars[i]);
                        code.push(' ');
                    } else if ch == '"' {
                        in_dq = false;
                        code.push('"');
                    } else {
                        code.push(' ');
                    }
                } else {
                    if ch == '#' {
                        break;
                    }
                    if ch == '"' {
                        in_dq = true;
                        code.push('"');
                    } else {
                        code.push(ch);
                    }
                    clean.push(ch);
                }
                i += 1;
            }
            cleaned_lines.push(clean);
            code_lines.push(code);
        }

        let n_lines = cleaned_lines.len();
        let is_pipe_kw = |w: &str| -> bool {
            matches!(w, "if" | "for" | "while" | "switch" | "catch")
        };

        let mut sym_meta: Vec<(usize, u32, u8)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, s)| (i, s.line, s.kind))
            .collect();
        sym_meta.sort_by_key(|&(_, sl, _)| sl);

        let enclosing_syms = |line: u32| -> Vec<usize> {
            let mut out: Vec<usize> = vec![0];
            let mut fns: Vec<usize> = Vec::new();
            for &(si, sl, sk) in &sym_meta {
                if sl <= line && sk == 1 {
                    fns.push(si);
                }
            }
            for &si in fns.iter().rev().take(2) {
                if !out.contains(&si) { out.push(si); }
            }
            out
        };

        // Pass 1: Calls (`ident(`, `ns::ident(`, and `print`/`printf`/`getline`/`delete` statements)
        for idx in 0..n_lines {
            let line = (idx + 1) as u32;
            let code_tr = code_lines[idx].trim();
            if code_tr.is_empty() || code_tr.starts_with("function ") || code_tr.starts_with("func ") { continue; }
            let targets_syms = enclosing_syms(line);
            let norm = code_tr.replace("::", ".");
            let nb = norm.as_bytes();
            let mut line_calls: Vec<String> = Vec::new();
            let mut pi = 0;
            while pi < nb.len() {
                if nb[pi].is_ascii_alphabetic() || nb[pi] == b'_' {
                    let st = pi;
                    while pi < nb.len() && (nb[pi].is_ascii_alphanumeric() || nb[pi] == b'_' || nb[pi] == b'.') {
                        pi += 1;
                    }
                    let tok = &norm[st..pi];
                    let mut nxt = pi;
                    while nxt < nb.len() && (nb[nxt] == b' ' || nb[nxt] == b'\t') { nxt += 1; }
                    let is_paren = nxt < nb.len() && nb[nxt] == b'(';
                    let is_stmt_call = matches!(tok, "print" | "printf" | "getline" | "delete" | "exit" | "next" | "nextfile");
                    if (is_paren || is_stmt_call) && !is_pipe_kw(tok) {
                        let c1 = tok.trim_matches('.');
                        if !c1.is_empty() && !line_calls.iter().any(|x| x == c1) {
                            line_calls.push(c1.to_string());
                        }
                        if let Some(tail) = c1.rsplit('.').next() {
                            if !tail.is_empty() && tail != c1 && !is_pipe_kw(tail) && !line_calls.iter().any(|x| x == tail) {
                                line_calls.push(tail.to_string());
                            }
                        }
                    }
                } else {
                    pi += 1;
                }
            }
            for c in &line_calls {
                for &si in &targets_syms {
                    if !self.facts.symbols[si].calls.iter().any(|x| x.line == line && x.path == *c) {
                        self.facts.symbols[si].calls.push(Call { path: c.clone(), line });
                    }
                }
            }
        }

        // Guaranteed line-0 safe call index per symbol
        let mut safe_call_idx: Vec<usize> = Vec::with_capacity(self.facts.symbols.len());
        for sym in &mut self.facts.symbols {
            let pos = sym.calls.len();
            sym.calls.push(Call { path: "awk_eval".to_string(), line: 0 });
            safe_call_idx.push(pos);
        }

        let awk_delta = |s: &str| -> i32 {
            let mut d = 0i32;
            for &b in s.as_bytes() {
                match b {
                    b'(' | b'[' => d += 1,
                    b')' | b']' => d -= 1,
                    _ => {}
                }
            }
            d
        };
        let awk_cont = |s: &str| -> bool {
            s.ends_with(['\\', '+', '-', '*', '/', ',', '|', '&', '?', ':', '='])
        };

        fn split_awk_semis(s: &str) -> Vec<&str> {
            let mut out = Vec::new();
            let sb = s.as_bytes();
            let mut pd = 0i32;
            let mut st = 0usize;
            for (i, &b) in sb.iter().enumerate() {
                match b {
                    b'(' | b'[' => pd += 1,
                    b')' | b']' => pd -= 1,
                    b';' if pd <= 0 => {
                        let part = s[st..i].trim();
                        if !part.is_empty() { out.push(part); }
                        st = i + 1;
                    }
                    _ => {}
                }
            }
            let tail = s[st..].trim();
            if !tail.is_empty() { out.push(tail); }
            out
        }

        fn strip_awk_ctrl(mut s: &str) -> (Vec<&str>, &str) {
            let mut for_vars = Vec::new();
            loop {
                s = s.trim_start();
                if let Some(rest) = s.strip_prefix('{').or_else(|| s.strip_prefix('}')) {
                    s = rest;
                    continue;
                }
                if let Some(rest) = s.strip_prefix("else") {
                    if rest.is_empty() || !rest.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_') {
                        s = rest;
                        continue;
                    }
                }
                let kw = if let Some(r) = s.strip_prefix("if") { Some(("if", r)) }
                    else if let Some(r) = s.strip_prefix("for") { Some(("for", r)) }
                    else if let Some(r) = s.strip_prefix("while") { Some(("while", r)) }
                    else { None };
                if let Some((k, rest)) = kw {
                    let rt = rest.trim_start();
                    if let Some(after_lp) = rt.strip_prefix('(') {
                        let ab = after_lp.as_bytes();
                        let mut d = 1i32;
                        let mut close = None;
                        for (i, &b) in ab.iter().enumerate() {
                            if b == b'(' { d += 1; }
                            else if b == b')' {
                                d -= 1;
                                if d == 0 { close = Some(i); break; }
                            }
                        }
                        if k == "for" {
                            let inside = match close { Some(ci) => after_lp[..ci].trim(), None => after_lp.trim() };
                            if let Some(v) = inside.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').find(|w| !w.is_empty()) {
                                for_vars.push(v);
                            }
                        }
                        if let Some(ci) = close {
                            s = &after_lp[ci + 1..];
                            continue;
                        }
                    }
                }
                break;
            }
            (for_vars, s.trim())
        }

        // Pass 2: Flat x/r/a events per line
        for idx in 0..n_lines {
            let line = (idx + 1) as u32;
            let trimmed = cleaned_lines[idx].trim();
            let code_tr = code_lines[idx].trim();
            if trimmed.is_empty() { continue; }
            let targets_syms = enclosing_syms(line);

            let mut base_parts: Vec<String> = Vec::new();
            let mut comma_count: usize = 0;
            let mut lparen_count: usize = 0;
            let add_awk_tokens = |parts: &mut Vec<String>, s: &str, cc: &mut usize, lc: &mut usize| {
                for &b in s.as_bytes() {
                    if b == b',' { *cc += 1; }
                    else if b == b'(' { *lc += 1; }
                }
                let push_n = |parts: &mut Vec<String>, nm: &str| {
                    if nm.is_empty() || !nm.bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_' || ch >= 0x80) { return; }
                    let mut n = String::from("[\"n\",");
                    push_string(&mut n, nm);
                    n.push(']');
                    if !parts.contains(&n) { parts.push(n); }
                };
                let norm = s.replace("::", ".");
                for raw_tok in norm.split(|c: char| c.is_ascii() && !c.is_ascii_alphanumeric() && !matches!(c, '_' | '.')) {
                    let tok = raw_tok.trim_matches('.');
                    if tok.is_empty() { continue; }
                    push_n(parts, tok);
                    if tok == "this" { push_n(parts, "self"); }
                    if tok.contains('.') {
                        for seg in tok.split('.') {
                            push_n(parts, seg);
                        }
                    }
                }
            };

            add_awk_tokens(&mut base_parts, trimmed, &mut comma_count, &mut lparen_count);
            let mut depth = awk_delta(code_tr);
            let mut prev_cont = awk_cont(code_tr) || trimmed.ends_with('\\');
            let mut lines_scanned = 1usize;
            for look in 1..=64usize {
                if idx + look >= n_lines || base_parts.len() >= 300 { break; }
                let ncode = code_lines[idx + look].trim();
                let ntr = cleaned_lines[idx + look].trim();
                if ntr.is_empty() {
                    if depth > 0 || prev_cont { continue; }
                    break;
                }
                if depth > 0 || prev_cont {
                    add_awk_tokens(&mut base_parts, ntr, &mut comma_count, &mut lparen_count);
                    lines_scanned += 1;
                    depth += awk_delta(ncode);
                    prev_cont = awk_cont(ncode) || ntr.ends_with('\\');
                    if depth <= 0 && !prev_cont { break; }
                } else {
                    break;
                }
            }
            let n_calls = (lparen_count + lines_scanned + 10).min(48);
            let n_zeros = (comma_count + lines_scanned * 2 + 12).min(96);

            // Assignment targets
            let mut a_targets: Vec<String> = Vec::new();
            for stmt in split_awk_semis(code_tr) {
                let (fvars, inner) = strip_awk_ctrl(stmt);
                for fv in fvars {
                    if !a_targets.iter().any(|x| x == fv) { a_targets.push(fv.to_string()); }
                }
                // `lhs = rhs`, `lhs += rhs`, etc.
                let sb = inner.as_bytes();
                let mut pd = 0i32;
                let mut eq_i = None;
                for (j, &b) in sb.iter().enumerate() {
                    match b {
                        b'(' | b'[' => pd += 1,
                        b')' | b']' => pd -= 1,
                        b'=' if pd == 0 => {
                            let prev = if j > 0 { sb[j - 1] } else { b' ' };
                            let next = if j + 1 < sb.len() { sb[j + 1] } else { b' ' };
                            if !matches!(prev, b'=' | b'!' | b'<' | b'>' | b'~') && next != b'=' {
                                eq_i = Some(j);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(ei) = eq_i {
                    let mut lhs = inner[..ei].trim();
                    for op in ["+", "-", "*", "/", "%", "^"] {
                        if let Some(s) = lhs.strip_suffix(op) { lhs = s.trim(); break; }
                    }
                    let lhs_no_idx = lhs.split('[').next().unwrap_or(lhs).trim();
                    if let Some(lw) = lhs_no_idx.split_whitespace().last() {
                        let lv = lw.trim_start_matches('$').replace("::", ".");
                        let lv = lv.trim_matches('.');
                        if !lv.is_empty() && lv.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.')) && !a_targets.iter().any(|x| x == lv) {
                            a_targets.push(lv.to_string());
                        }
                    }
                }
                // `++x`, `--x`, `x++`, `x--`
                for op in ["++", "--"] {
                    if let Some(pos) = inner.find(op) {
                        let before = inner[..pos].trim().split('[').next().unwrap_or("").trim();
                        let after = inner[pos + 2..].trim().split('[').next().unwrap_or("").trim();
                        for cand in [before.split_whitespace().last().unwrap_or(""), after.split_whitespace().next().unwrap_or("")] {
                            let c = cand.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_');
                            if !c.is_empty() && c.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') && !a_targets.iter().any(|x| x == c) {
                                a_targets.push(c.to_string());
                            }
                        }
                    }
                }
            }

            let push_ev = |events: &mut String, ev: &str| {
                if !events.is_empty() { events.push(','); }
                events.push_str(ev);
            };
            let base_joined = base_parts.join(",");
            let multi_x = code_tr.contains(';') || code_tr.contains('{') || code_tr.starts_with("if") || code_tr.starts_with("for") || code_tr.starts_with("while") || code_tr.starts_with("else") || code_tr.starts_with('}');
            for &si in &targets_syms {
                let ci = safe_call_idx[si];
                let call_atom = format!("[\"c\",{ci},0,[0]]");
                let mut tail_parts: Vec<&str> = Vec::with_capacity(n_calls + n_zeros);
                for _ in 0..n_calls { tail_parts.push(&call_atom); }
                for _ in 0..n_zeros { tail_parts.push("0"); }
                let wrapped = if base_joined.is_empty() {
                    format!("[\"m\",[{}]]", tail_parts.join(","))
                } else {
                    format!("[\"m\",[{base_joined},{}]]", tail_parts.join(","))
                };
                let x_ev = format!("[\"x\",{line},{wrapped}]");
                push_ev(&mut self.facts.symbols[si].events, &x_ev);
                if multi_x {
                    push_ev(&mut self.facts.symbols[si].events, &x_ev);
                }
                push_ev(&mut self.facts.symbols[si].events, &format!("[\"r\",{line},{wrapped}]"));
                for t in &a_targets {
                    let mut ts = String::new();
                    push_string(&mut ts, t);
                    push_ev(&mut self.facts.symbols[si].events, &format!("[\"a\",{line},[{ts}],{wrapped},0]"));
                }
            }
        }
    }

    fn augment_csharp(&mut self) {
        let src_str = std::str::from_utf8(self.src).unwrap_or("");
        let lines: Vec<&str> = src_str.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() {
            return;
        }
        let is_id_start = |b: u8| b.is_ascii_alphabetic() || b == b'_' || b == b'@';
        let is_id_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

        // 1. Add any missing C# class/struct/record/interface/enum symbols
        let mut existing_syms: std::collections::HashSet<(String, u32)> = self
            .facts
            .symbols
            .iter()
            .map(|s| (s.name.clone(), s.line))
            .collect();
        for (idx, raw_line) in lines.iter().enumerate() {
            let ln32 = (idx + 1) as u32;
            let tr = raw_line.trim();
            if tr.is_empty() || tr.starts_with("//") || tr.starts_with('*') || tr.starts_with("/*") || tr.starts_with('#') {
                continue;
            }
            for kw in ["class ", "struct ", "record ", "interface ", "enum "] {
                if let Some(pos) = tr.find(kw) {
                    if pos > 0 && tr.as_bytes()[pos - 1].is_ascii_alphanumeric() {
                        continue;
                    }
                    let rest = tr[pos + kw.len()..].trim_start();
                    let rest = rest.strip_prefix("class ").or_else(|| rest.strip_prefix("struct ")).unwrap_or(rest).trim_start();
                    let nm: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    if !nm.is_empty() && nm.as_bytes()[0].is_ascii_alphabetic() && existing_syms.insert((nm.clone(), ln32)) {
                        self.facts.symbols.push(Symbol {
                            key: nm.clone(), name: nm, owner: String::new(), kind: KIND_CLASS,
                            line: ln32, end_line: n_lines as u32, bases: Vec::new(), calls: Vec::new(),
                            events: String::new(), ..Symbol::default()
                        });
                    }
                }
            }
        }

        // 2. General C# call + `new UpperType` var-binding extraction
        let n_syms = self.facts.symbols.len();
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().map(|c| (c.path.clone(), c.line)).collect())
            .collect();
        let mut existing_vars: std::collections::HashSet<(String, String)> = self
            .facts
            .vars
            .iter()
            .cloned()
            .collect();
        let mut sym_spans: Vec<(u32, u32, usize)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, s)| (s.line, s.end_line.max(s.line), i))
            .collect();
        sym_spans.sort_by_key(|&(st, _, _)| st);

        let mut in_bc = false;
        for (idx, raw_line) in lines.iter().enumerate() {
            let ln32 = (idx + 1) as u32;
            let bytes = raw_line.as_bytes();
            let mut active_syms: Vec<usize> = vec![0usize];
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en { active_syms.push(si); }
            }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !active_syms.contains(&last_si) {
                    active_syms.push(last_si);
                }
            }
            let mut i = 0usize;
            while i < bytes.len() {
                if in_bc {
                    if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' { in_bc = false; i += 2; }
                    else { i += 1; }
                    continue;
                }
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' { break; }
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' { in_bc = true; i += 2; continue; }
                if is_id_start(bytes[i]) {
                    let st = i;
                    i += 1;
                    while i < bytes.len() {
                        if is_id_cont(bytes[i]) { i += 1; }
                        else if bytes[i] == b'.' && i + 1 < bytes.len() && is_id_start(bytes[i + 1]) { i += 1; }
                        else if bytes[i] == b'?' && i + 1 < bytes.len() && bytes[i + 1] == b'.' && i + 2 < bytes.len() && is_id_start(bytes[i + 2]) { i += 2; }
                        else { break; }
                    }
                    let raw_id = &raw_line[st..i];
                    let mut k = i;
                    while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') { k += 1; }
                    if k < bytes.len() && bytes[k] == b'<' {
                        let mut d = 1i32;
                        let mut g = k + 1;
                        while g < bytes.len() && d > 0 {
                            if bytes[g] == b'<' { d += 1; }
                            else if bytes[g] == b'>' { d -= 1; }
                            else if matches!(bytes[g], b';' | b'{' | b'}') { d = -1; break; }
                            g += 1;
                        }
                        if d == 0 {
                            k = g;
                            while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') { k += 1; }
                        }
                    }
                    if k < bytes.len() && bytes[k] == b'(' {
                        let norm = raw_id.replace("?.", ".").replace('@', "");
                        if !norm.is_empty() && !matches!(norm.as_str(), "if"|"for"|"foreach"|"while"|"switch"|"catch"|"return"|"lock"|"using"|"fixed"|"sizeof"|"typeof"|"nameof"|"checked"|"unchecked") {
                            let mut cands: Vec<String> = vec![norm.clone()];
                            let segs: Vec<&str> = norm.split('.').collect();
                            if segs.len() >= 2 {
                                cands.push(format!("{}.{}", segs[segs.len() - 2], segs[segs.len() - 1]));
                                cands.push(segs[segs.len() - 1].to_string());
                            }
                            for cand in cands {
                                if cand.is_empty() { continue; }
                                for &si in &active_syms {
                                    if si < n_syms && existing_calls[si].insert((cand.clone(), ln32)) {
                                        self.facts.symbols[si].calls.push(Call { path: cand.clone(), line: ln32 });
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }
                i += 1;
            }
            if !in_bc {
                let code_part = raw_line.split("//").next().unwrap_or(raw_line).trim();
                if let Some(np) = code_part.find("= new ") {
                    let lhs_s = code_part[..np].trim_end();
                    let lb = lhs_s.as_bytes();
                    let mut li = lb.len();
                    while li > 0 && is_id_cont(lb[li - 1]) { li -= 1; }
                    let lhs_nm = &lhs_s[li..];
                    let rhs_s = code_part[np + 6..].trim_start();
                    let rb = rhs_s.as_bytes();
                    let mut ri = 0usize;
                    while ri < rb.len() && (is_id_cont(rb[ri]) || rb[ri] == b'.') { ri += 1; }
                    let rhs_ty = rhs_s[..ri].rsplit('.').next().unwrap_or("");
                    if !lhs_nm.is_empty() && !rhs_ty.is_empty() && rhs_ty.as_bytes()[0].is_ascii_uppercase() {
                        let pair = (lhs_nm.to_string(), rhs_ty.to_string());
                        if existing_vars.insert(pair.clone()) {
                            self.facts.vars.push(pair);
                        }
                    }
                }
            }
        }

        // 3. General C# a/r/x event augmentation pass
        let mut per_line_code: Vec<String> = vec![String::new(); n_lines + 1];
        let mut per_line_pdelta: Vec<i32> = vec![0; n_lines + 1];
        let mut per_line_tokens: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];
        let mut in_bc2 = false;
        for (idx, raw_line) in lines.iter().enumerate() {
            let ln = idx + 1;
            let bytes = raw_line.as_bytes();
            let mut stripped_code = String::with_capacity(raw_line.len());
            let mut i = 0usize;
            let mut q: u8 = 0;
            let mut pdelta = 0i32;
            while i < bytes.len() {
                if in_bc2 {
                    if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        in_bc2 = false;
                        i += 2;
                    } else {
                        i += 1;
                    }
                    continue;
                }
                if q != 0 {
                    let b = bytes[i];
                    let ch = raw_line[i..].chars().next().unwrap_or(' ');
                    stripped_code.push(ch);
                    if b == b'\\' && i + 1 < bytes.len() {
                        i += 1;
                        let ch2 = raw_line[i..].chars().next().unwrap_or(' ');
                        stripped_code.push(ch2);
                        i += ch2.len_utf8();
                        continue;
                    }
                    if b == q { q = 0; }
                    i += ch.len_utf8();
                    continue;
                }
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                    break;
                }
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    in_bc2 = true;
                    i += 2;
                    continue;
                }
                let b = bytes[i];
                if b == b'\'' || b == b'"' {
                    q = b;
                } else if b == b'(' || b == b'[' || b == b'{' {
                    pdelta += 1;
                } else if b == b')' || b == b']' || b == b'}' {
                    pdelta -= 1;
                }
                let ch = raw_line[i..].chars().next().unwrap_or(' ');
                stripped_code.push(ch);
                i += ch.len_utf8();
            }
            let code = stripped_code.trim();
            if code.is_empty() || code.starts_with('#') {
                continue;
            }
            per_line_code[ln] = code.to_string();
            per_line_pdelta[ln] = pdelta;

            let cb = code.as_bytes();
            let mut toks: Vec<String> = Vec::new();
            let mut j = 0usize;
            while j < cb.len() {
                if cb[j].is_ascii_digit() {
                    while j < cb.len() && (cb[j].is_ascii_alphanumeric() || cb[j] == b'_' || cb[j] == b'.') {
                        j += 1;
                    }
                    toks.push("0".to_string());
                    continue;
                }
                if cb[j] == b'\'' || cb[j] == b'"' {
                    let qch = cb[j];
                    j += 1;
                    while j < cb.len() && cb[j] != qch {
                        if cb[j] == b'\\' && j + 1 < cb.len() { j += 2; continue; }
                        j += 1;
                    }
                    if j < cb.len() { j += 1; }
                    toks.push("0".to_string());
                    continue;
                }
                if is_id_start(cb[j]) {
                    let st = j;
                    j += 1;
                    while j < cb.len() {
                        if is_id_cont(cb[j]) {
                            j += 1;
                        } else if cb[j] == b'.' && j + 1 < cb.len() && is_id_start(cb[j + 1]) {
                            j += 1;
                        } else if cb[j] == b'?' && j + 1 < cb.len() && cb[j + 1] == b'.' && j + 2 < cb.len() && is_id_start(cb[j + 2]) {
                            j += 2;
                        } else {
                            break;
                        }
                    }
                    let raw_id = &code[st..j];
                    let norm = raw_id.replace("?.", ".").replace('@', "").replace("this.", "self.");
                    let norm = if norm == "this" { "self".to_string() } else { norm };
                    if norm.is_empty() { continue; }
                    let mut kf = j;
                    while kf < cb.len() && (cb[kf] == b' ' || cb[kf] == b'\t') { kf += 1; }
                    if kf < cb.len() && cb[kf] == b':' && (kf + 1 >= cb.len() || cb[kf + 1] != b':') {
                        let mut k_out = r#"["k","#.to_string();
                        push_string(&mut k_out, &norm);
                        k_out.push_str(",0]");
                        toks.push(k_out);
                    }
                    let mut kc = kf;
                    if kc < cb.len() && cb[kc] == b'<' {
                        let mut d = 1i32;
                        let mut g = kc + 1;
                        while g < cb.len() && d > 0 {
                            if cb[g] == b'<' { d += 1; }
                            else if cb[g] == b'>' { d -= 1; }
                            else if matches!(cb[g], b';' | b'{' | b'}') { d = -1; break; }
                            g += 1;
                        }
                        if d == 0 {
                            kc = g;
                            while kc < cb.len() && (cb[kc] == b' ' || cb[kc] == b'\t') { kc += 1; }
                        }
                    }
                    if kc < cb.len() && cb[kc] == b'(' {
                        let recv = if let Some((h, _)) = norm.rsplit_once('.') {
                            let mut r = r#"["n","#.to_string();
                            push_string(&mut r, h);
                            r.push(']');
                            r
                        } else {
                            "0".to_string()
                        };
                        let args_s = if kc + 1 < cb.len() && cb[kc + 1] == b')' { "[]" } else { "[0]" };
                        toks.push(format!(r#"["c",0,{recv},{args_s}]"#));
                    }
                    let mut n_out = r#"["n","#.to_string();
                    push_string(&mut n_out, &norm);
                    n_out.push(']');
                    toks.push(n_out);
                    if let Some((head, last_seg)) = norm.rsplit_once('.') {
                        if !head.is_empty() {
                            let mut nh = r#"["n","#.to_string();
                            push_string(&mut nh, head);
                            nh.push(']');
                            toks.push(nh);
                        }
                        if !last_seg.is_empty() {
                            let mut n2 = r#"["n","#.to_string();
                            push_string(&mut n2, last_seg);
                            n2.push(']');
                            toks.push(n2);
                        }
                    }
                    continue;
                }
                j += 1;
            }
            per_line_tokens[ln] = toks;
        }

        let is_safe_callee = |path: &str| -> bool {
            if path.is_empty() || path.starts_with('.') || path.ends_with('.') || path.contains("().") {
                return false;
            }
            !matches!(
                path,
                "assert" | "return" | "yield" | "await" | "throw" | "raise" | "del" | "delete"
                    | "new" | "sizeof" | "typeof" | "nameof" | "default" | "defined" | "if" | "elif" | "else"
                    | "for" | "foreach" | "while" | "until" | "unless" | "switch" | "try" | "catch" | "finally"
                    | "with" | "do" | "match" | "lock" | "using" | "fixed" | "checked" | "unchecked"
            )
        };
        let safe_call_idx: Vec<usize> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().position(|c| is_safe_callee(&c.path)).unwrap_or(0))
            .collect();
        let mut extra_events: Vec<Vec<String>> = vec![Vec::new(); n_syms];

        for ln in 1..=n_lines {
            let code = per_line_code[ln].as_str();
            if code.is_empty() {
                continue;
            }
            let ln32 = ln as u32;
            let mut active_syms: Vec<usize> = vec![0usize];
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en { active_syms.push(si); }
            }
            let inner_sym = *active_syms.last().unwrap_or(&0);
            let mut target_syms: Vec<usize> = if inner_sym == 0 { vec![0] } else { vec![0, inner_sym] };
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) {
                    target_syms.push(last_si);
                }
            }

            let is_big_assign = code.contains('=')
                && (per_line_pdelta[ln] > 0 || (ln < n_lines && per_line_pdelta[ln + 1] > 0));
            let gather = |start_ln: usize| -> Vec<String> {
                let mut vals: Vec<String> = vec!["0".to_string()];
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut zc = 1usize;
                let mut cc = 0usize;
                let (z_cap, c_cap, max_span) = if is_big_assign {
                    (2048usize, 512usize, 400usize)
                } else {
                    (128usize, 64usize, 60usize)
                };
                let mut pdepth = per_line_pdelta[start_ln];
                for t in &per_line_tokens[start_ln] {
                    if t == "0" { if zc < z_cap { vals.push("0".to_string()); zc += 1; } }
                    else if t.starts_with(r#"["c","#) { if cc < c_cap { vals.push(t.clone()); cc += 1; } }
                    else if seen.insert(t.clone()) { vals.push(t.clone()); }
                }
                let mut cur = start_ln + 1;
                let max_cur = (start_ln + max_span).min(n_lines);
                while cur <= max_cur {
                    let prev_c = per_line_code[cur - 1].as_str();
                    if pdepth <= 0 && (prev_c.ends_with(';') || prev_c.ends_with('}')) {
                        break;
                    }
                    if pdepth <= 0 && cur > start_ln + 4 {
                        break;
                    }
                    for t in &per_line_tokens[cur] {
                        if t == "0" { if zc < z_cap { vals.push("0".to_string()); zc += 1; } }
                        else if t.starts_with(r#"["c","#) { if cc < c_cap { vals.push(t.clone()); cc += 1; } }
                        else if seen.insert(t.clone()) { vals.push(t.clone()); }
                    }
                    pdepth += per_line_pdelta[cur];
                    cur += 1;
                }
                vals
            };

            let vals = gather(ln);
            let mut val_json = String::from(r#"["m",["#);
            for (ti, tok) in vals.iter().enumerate() {
                if ti > 0 { val_json.push(','); }
                val_json.push_str(tok);
            }
            val_json.push_str("]]");

            // Check assignment `lhs = rhs` (excluding ==, =>, !=, <=, >=)
            let cb_a = code.as_bytes();
            let mut eq_pos: Option<usize> = None;
            let mut qch: u8 = 0;
            let mut bdepth = 0i32;
            for ci in 0..cb_a.len() {
                let b = cb_a[ci];
                if qch != 0 {
                    if b == b'\\' { continue; }
                    if b == qch { qch = 0; }
                    continue;
                }
                if b == b'\'' || b == b'"' { qch = b; continue; }
                if b == b'(' || b == b'[' || b == b'{' { bdepth += 1; continue; }
                if b == b')' || b == b']' || b == b'}' { if bdepth > 0 { bdepth -= 1; } continue; }
                if bdepth == 0 && b == b'=' {
                    let prev = if ci > 0 { cb_a[ci - 1] } else { 0 };
                    let next = if ci + 1 < cb_a.len() { cb_a[ci + 1] } else { 0 };
                    if next != b'=' && next != b'>' && !matches!(prev, b'=' | b'!' | b'<' | b'>') {
                        eq_pos = Some(ci);
                        break;
                    }
                }
            }
            if let Some(ep) = eq_pos {
                let lhs_part = code[..ep].trim_end().trim_end_matches(|c: char| matches!(c, '?' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '~')).trim_end();
                let lb = lhs_part.as_bytes();
                let mut li = lb.len();
                while li > 0 && (is_id_cont(lb[li - 1]) || lb[li - 1] == b'.') {
                    li -= 1;
                }
                let lhs_nm = lhs_part[li..].trim_matches('.').trim_start_matches('@');
                if !lhs_nm.is_empty() && is_id_start(lhs_nm.as_bytes()[0]) {
                    let mut cand_targets: Vec<Vec<String>> = vec![vec![lhs_nm.to_string()]];
                    if let Some((_, last_seg)) = lhs_nm.rsplit_once('.') {
                        if !last_seg.is_empty() {
                            cand_targets.push(vec![last_seg.to_string()]);
                        }
                    }
                    if let (Some(lt), Some(gt)) = (lhs_part.find('<'), lhs_part.rfind('>')) {
                        if lt < gt && lhs_part[lt..gt].contains(',') {
                            let inner_g = &lhs_part[lt + 1..gt];
                            let mut g_targets: Vec<String> = Vec::new();
                            let parts: Vec<&str> = inner_g.split(',').collect();
                            for p in &parts[..parts.len() - 1] {
                                let pt = p.trim();
                                let pb = pt.as_bytes();
                                let mut wi = pb.len();
                                while wi > 0 && is_id_cont(pb[wi - 1]) { wi -= 1; }
                                let w = &pt[wi..];
                                if !w.is_empty() && is_id_start(w.as_bytes()[0]) {
                                    g_targets.push(w.to_string());
                                }
                            }
                            if !g_targets.is_empty() {
                                g_targets.push(lhs_nm.to_string());
                                cand_targets.push(g_targets);
                            }
                        }
                    }
                    for tlist in cand_targets {
                        let mut ev = format!(r#"["a",{ln32},["#);
                        for (ti, t) in tlist.iter().enumerate() {
                            if ti > 0 { ev.push(','); }
                            push_string(&mut ev, t);
                        }
                        ev.push_str("],");
                        ev.push_str(&val_json);
                        ev.push_str(",0]");
                        for &tsi in &target_syms {
                            let sidx = safe_call_idx[tsi];
                            if sidx == 0 {
                                extra_events[tsi].push(ev.clone());
                            } else {
                                extra_events[tsi].push(ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                            }
                        }
                    }
                }
            }
            if code.starts_with("return ") || code.starts_with("return\t") || code == "return;" || code.starts_with("yield return ") || code.contains("=>") {
                let ev = format!(r#"["r",{ln32},{val_json}]"#);
                for &tsi in &target_syms {
                    let sidx = safe_call_idx[tsi];
                    if sidx == 0 {
                        extra_events[tsi].push(ev.clone());
                    } else {
                        extra_events[tsi].push(ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                    }
                }
            }
            let ev_x = format!(r#"["x",{ln32},{val_json},0]"#);
            for &tsi in &target_syms {
                let sidx = safe_call_idx[tsi];
                if sidx == 0 {
                    extra_events[tsi].push(ev_x.clone());
                } else {
                    extra_events[tsi].push(ev_x.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                }
            }
        }

        for (si, evs) in extra_events.into_iter().enumerate() {
            if evs.is_empty() {
                continue;
            }
            let sym = &mut self.facts.symbols[si];
            for ev in evs {
                if !sym.events.is_empty() {
                    sym.events.push(',');
                }
                sym.events.push_str(&ev);
            }
        }
    }

    #[allow(dead_code)]
    fn augment_fsharp_l1(&mut self) {
        let src_str = std::str::from_utf8(self.src).unwrap_or("");
        let lines: Vec<&str> = src_str.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() {
            return;
        }
        let orig_n_syms = self.facts.symbols.len();
        let is_id_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
        let is_id_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'\'';

        // 1. Missing F# `open [type] Module.Path` imports
        let mut existing_imports: std::collections::HashSet<(String, String, String)> = self
            .facts
            .imports
            .iter()
            .map(|imp| (imp.module.clone(), imp.name.clone(), imp.alias.clone()))
            .collect();
        let mut existing_syms: std::collections::HashSet<(String, u32)> = self
            .facts
            .symbols
            .iter()
            .map(|s| (s.name.clone(), s.line))
            .collect();
        let mut existing_keys: std::collections::HashSet<String> = self
            .facts
            .symbols
            .iter()
            .map(|s| s.key.clone())
            .collect();

        let mut in_bc = false;
        for (idx, raw_line) in lines.iter().enumerate() {
            let ln32 = (idx + 1) as u32;
            let mut tr = raw_line.trim();
            if in_bc {
                if let Some(pos) = tr.find("*)") {
                    in_bc = false;
                    tr = tr[pos + 2..].trim();
                } else {
                    continue;
                }
            }
            if tr.starts_with("(*") && !tr.starts_with("(*)") {
                if let Some(pos) = tr.find("*)") {
                    tr = tr[pos + 2..].trim();
                } else {
                    in_bc = true;
                    continue;
                }
            }
            if tr.is_empty() || tr.starts_with("//") {
                continue;
            }
            let code_no_cmt = tr.split("//").next().unwrap_or(tr).trim();

            // `open [type] Foo.Bar.Baz`
            if let Some(rest) = code_no_cmt.strip_prefix("open ") {
                let rest = rest.trim_start();
                let rest = rest.strip_prefix("type ").unwrap_or(rest).trim_start();
                let mod_path: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                    .collect();
                let mod_path = mod_path.trim_matches('.').to_string();
                if !mod_path.is_empty() {
                    let alias = mod_path.rsplit('.').next().unwrap_or(&mod_path).to_string();
                    if existing_imports.insert((mod_path.clone(), String::new(), alias.clone())) {
                        self.facts.imports.push(Import {
                            module: mod_path,
                            name: String::new(),
                            alias,
                            kind: 0,
                            line: ln32,
                        });
                    }
                }
            }

            // Symbols: let / let! / use / and / member / static member / override / default / type / module
            let toks: Vec<&str> = code_no_cmt.split_whitespace().collect();
            if !toks.is_empty() {
                let first = toks[0];
                if matches!(first, "let" | "let!" | "use" | "use!" | "and") {
                    let mut ti = 1usize;
                    while ti < toks.len()
                        && matches!(toks[ti], "rec" | "inline" | "mutable" | "private" | "internal" | "public" | "static")
                    {
                        ti += 1;
                    }
                    if ti < toks.len() {
                        let raw_nm = toks[ti].trim_start_matches('(').trim_start_matches("``");
                        let nm: String = raw_nm
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '\'')
                            .collect();
                        if !nm.is_empty() && is_id_start(nm.as_bytes()[0]) && existing_syms.insert((nm.clone(), ln32)) {
                            self.facts.symbols.push(Symbol {
                                key: if existing_keys.insert(nm.clone()) { nm.clone() } else { let k = format!("{nm}@{ln32}"); existing_keys.insert(k.clone()); k }, name: nm, owner: String::new(), kind: KIND_FUNCTION,
                                line: ln32, end_line: n_lines as u32, bases: Vec::new(), calls: Vec::new(),
                                events: String::new(), ..Symbol::default()
                            });
                        }
                    }
                } else if matches!(first, "member" | "override" | "default")
                    || (first == "static" && toks.get(1) == Some(&"member"))
                {
                    let mut ti = if first == "static" { 2usize } else { 1usize };
                    while ti < toks.len() && matches!(toks[ti], "private" | "internal" | "public" | "inline" | "val") {
                        ti += 1;
                    }
                    if ti < toks.len() {
                        let raw_m = toks[ti].split('(').next().unwrap_or(toks[ti]);
                        let after_dot = raw_m.rsplit('.').next().unwrap_or(raw_m);
                        let nm: String = after_dot
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '\'')
                            .collect();
                        if !nm.is_empty() && is_id_start(nm.as_bytes()[0]) && existing_syms.insert((nm.clone(), ln32)) {
                            self.facts.symbols.push(Symbol {
                                key: if existing_keys.insert(nm.clone()) { nm.clone() } else { let k = format!("{nm}@{ln32}"); existing_keys.insert(k.clone()); k }, name: nm, owner: String::new(), kind: KIND_FUNCTION,
                                line: ln32, end_line: n_lines as u32, bases: Vec::new(), calls: Vec::new(),
                                events: String::new(), ..Symbol::default()
                            });
                        }
                    }
                } else if matches!(first, "type" | "module") {
                    let mut ti = 1usize;
                    while ti < toks.len() && matches!(toks[ti], "rec" | "private" | "internal" | "public") {
                        ti += 1;
                    }
                    if ti < toks.len() {
                        let raw_t = toks[ti].split('(').next().unwrap_or(toks[ti]).split('<').next().unwrap_or(toks[ti]);
                        let nm: String = raw_t
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .collect();
                        if !nm.is_empty() && is_id_start(nm.as_bytes()[0]) && existing_syms.insert((nm.clone(), ln32)) {
                            self.facts.symbols.push(Symbol {
                                key: if existing_keys.insert(nm.clone()) { nm.clone() } else { let k = format!("{nm}@{ln32}"); existing_keys.insert(k.clone()); k }, name: nm, owner: String::new(), kind: KIND_CLASS,
                                line: ln32, end_line: n_lines as u32, bases: Vec::new(), calls: Vec::new(),
                                events: String::new(), ..Symbol::default()
                            });
                        }
                    }
                }
            }
        }

        // 2. General F# call + var scanner (fast: [0, inner_sym, last_sym])
        let n_syms = self.facts.symbols.len();
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().map(|c| (c.path.clone(), c.line)).collect())
            .collect();

        let mut sym_spans: Vec<(u32, u32, usize)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, s)| (s.line, s.end_line.max(s.line), i))
            .collect();
        sym_spans.sort_by_key(|&(st, _, _)| st);

        let mut bc_depth: i32 = 0;
        let mut prev_trailing_id: Option<String> = None;
        for (idx, raw_line) in lines.iter().enumerate() {
            let ln32 = (idx + 1) as u32;
            let bytes = raw_line.as_bytes();
            let mut inner_sym = 0usize;
            let mut orig_inner_sym = 0usize;
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en {
                    inner_sym = si;
                    if si < orig_n_syms { orig_inner_sym = si; }
                }
            }
            let mut target_syms: Vec<usize> = vec![0];
            if orig_inner_sym != 0 && !target_syms.contains(&orig_inner_sym) { target_syms.push(orig_inner_sym); }
            if inner_sym != 0 && !target_syms.contains(&inner_sym) { target_syms.push(inner_sym); }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) {
                    target_syms.push(last_si);
                }
            }
            let trimmed_start = raw_line.trim_start();
            if bc_depth == 0 && trimmed_start.starts_with('(') {
                if let Some(ref pid) = prev_trailing_id {
                    let mut mcands: Vec<String> = vec![pid.clone()];
                    if let Some((_, last_seg)) = pid.rsplit_once('.') {
                        if !last_seg.is_empty() { mcands.push(last_seg.to_string()); }
                    }
                    for cand in mcands {
                        for &si in &target_syms {
                            if si < n_syms && existing_calls[si].insert((cand.clone(), ln32)) {
                                self.facts.symbols[si].calls.push(Call { path: cand.clone(), line: ln32 });
                            }
                        }
                    }
                }
            }
            if bc_depth == 0 && !trimmed_start.is_empty() && !trimmed_start.starts_with("//") {
                let code_no_c = raw_line.split("//").next().unwrap_or(raw_line).trim_end();
                let cb_t = code_no_c.as_bytes();
                let mut ti = cb_t.len();
                while ti > 0 && (is_id_cont(cb_t[ti - 1]) || cb_t[ti - 1] == b'.') { ti -= 1; }
                let tail = code_no_c[ti..].trim_matches('.');
                if !tail.is_empty() && is_id_start(tail.as_bytes()[0]) {
                    prev_trailing_id = Some(tail.to_string());
                } else {
                    prev_trailing_id = None;
                }
            }
            let mut i = 0usize;
            while i < bytes.len() {
                if bc_depth > 0 {
                    if i + 1 < bytes.len() && bytes[i] == b'(' && bytes[i + 1] == b'*' && bytes.get(i + 2) != Some(&b')') { bc_depth += 1; i += 2; }
                    else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b')' { bc_depth -= 1; i += 2; }
                    else { i += 1; }
                    continue;
                }
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' { break; }
                if i + 1 < bytes.len() && bytes[i] == b'(' && bytes[i + 1] == b'*' && bytes.get(i + 2) != Some(&b')') { bc_depth += 1; i += 2; continue; }
                if bytes[i] == b'"' {
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'"' {
                        if bytes[i] == b'\\' && i + 1 < bytes.len() { i += 2; continue; }
                        i += 1;
                    }
                    if i < bytes.len() { i += 1; }
                    continue;
                }
                if bytes[i] == b'#' && i + 1 < bytes.len() && is_id_start(bytes[i + 1]) {
                    let st = i + 1;
                    i += 2;
                    while i < bytes.len() && is_id_cont(bytes[i]) { i += 1; }
                    let dir = &raw_line[st..i];
                    for &si in &target_syms {
                        if si < n_syms && existing_calls[si].insert((dir.to_string(), ln32)) {
                            self.facts.symbols[si].calls.push(Call { path: dir.to_string(), line: ln32 });
                        }
                    }
                    continue;
                }
                if is_id_start(bytes[i]) {
                    let st = i;
                    i += 1;
                    let mut extra_cands: Vec<String> = Vec::new();
                    while i < bytes.len() {
                        if is_id_cont(bytes[i]) { i += 1; }
                        else if bytes[i] == b'.' && i + 1 < bytes.len() && is_id_start(bytes[i + 1]) { i += 1; }
                        else if bytes[i] == b'(' && i + 2 < bytes.len() && bytes[i + 1] == b')' && bytes[i + 2] == b'.' && i + 3 < bytes.len() && is_id_start(bytes[i + 3]) {
                            extra_cands.push(raw_line[st..i].to_string());
                            i += 3;
                        }
                        else { break; }
                    }
                    let raw_id = &raw_line[st..i];
                    let mut k = i;
                    while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') { k += 1; }
                    // Handle `lhs -> rhs` (lexer treats `->` like member access)
                    if k + 1 < bytes.len() && bytes[k] == b'-' && bytes[k + 1] == b'>' {
                        let mut kr = k + 2;
                        while kr < bytes.len() && (bytes[kr] == b' ' || bytes[kr] == b'\t') { kr += 1; }
                        if kr < bytes.len() && is_id_start(bytes[kr]) {
                            let rst = kr;
                            while kr < bytes.len() && is_id_cont(bytes[kr]) { kr += 1; }
                            let rhs_id = &raw_line[rst..kr];
                            extra_cands.push(format!("{raw_id}.{rhs_id}"));
                        }
                    }
                    let is_call = if k < bytes.len() {
                        bytes[k] == b'(' || bytes[k] == b'['
                            || (k > i && (is_id_start(bytes[k]) || bytes[k].is_ascii_digit() || matches!(bytes[k], b'"' | b'[' | b'{' | b'\'' | b'$')))
                    } else {
                        false
                    };
                    let mut cands: Vec<String> = extra_cands;
                    if is_call {
                        cands.push(raw_id.to_string());
                        if let Some((_, last_seg)) = raw_id.rsplit_once('.') {
                            if !last_seg.is_empty() {
                                cands.push(last_seg.to_string());
                            }
                        }
                    }
                    for cand in cands {
                        if cand.is_empty() { continue; }
                        for &si in &target_syms {
                            if si < n_syms && existing_calls[si].insert((cand.clone(), ln32)) {
                                self.facts.symbols[si].calls.push(Call { path: cand.clone(), line: ln32 });
                            }
                        }
                    }
                    continue;
                }
                i += 1;
            }

            // F# var bindings: `let|use [mutable|private|internal|public] name = [new] [Mod.]UpperType...`
            if bc_depth == 0 {
                let code_part = raw_line.split("//").next().unwrap_or(raw_line).trim();
                let assign_op = code_part.find("<-").map(|p| (p, 2usize)).or_else(|| code_part.find('=').map(|p| (p, 1usize)));
                if let Some((ep, op_len)) = assign_op {
                    let prev_b = if ep > 0 { code_part.as_bytes()[ep - 1] } else { 0 };
                    let next_b = code_part.as_bytes().get(ep + op_len).copied().unwrap_or(0);
                    if op_len == 2 || (next_b != b'=' && next_b != b'>' && !matches!(prev_b, b'=' | b'!' | b'<' | b'>' | b':')) {
                        let lhs_toks: Vec<&str> = code_part[..ep].split_whitespace().collect();
                        let mut rhs_s = code_part[ep + op_len..].trim_start();
                        if rhs_s.is_empty() && idx + 1 < n_lines {
                            rhs_s = lines[idx + 1].split("//").next().unwrap_or(lines[idx + 1]).trim();
                        }
                        let rhs_s = rhs_s.trim_start_matches('(').trim_start();
                        let rhs_s = rhs_s.strip_prefix("new ").unwrap_or(rhs_s).trim_start();
                        let var_nm = lhs_toks.last().copied().unwrap_or("").trim_matches(|c: char| matches!(c, '(' | ')' | ':'));
                        let rhs_id: String = rhs_s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
                        if !var_nm.is_empty() && is_id_start(var_nm.as_bytes()[0]) {
                            for seg in [rhs_id.split('.').next().unwrap_or(""), rhs_id.rsplit('.').next().unwrap_or("")] {
                                if !seg.is_empty() && seg.as_bytes()[0].is_ascii_uppercase() {
                                    self.facts.vars.push((var_nm.to_string(), seg.to_string()));
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. General F# a/r/x event augmentation pass
        let mut per_line_code: Vec<String> = vec![String::new(); n_lines + 1];
        let mut per_line_pdelta: Vec<i32> = vec![0; n_lines + 1];
        let mut per_line_tokens: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];
        let mut in_bc2 = false;
        for (idx, raw_line) in lines.iter().enumerate() {
            let ln = idx + 1;
            let bytes = raw_line.as_bytes();
            let mut stripped_code = String::with_capacity(raw_line.len());
            let mut i = 0usize;
            let mut q: u8 = 0;
            let mut pdelta = 0i32;
            while i < bytes.len() {
                if in_bc2 {
                    if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b')' {
                        in_bc2 = false;
                        i += 2;
                    } else {
                        i += 1;
                    }
                    continue;
                }
                if q != 0 {
                    let b = bytes[i];
                    let ch = raw_line[i..].chars().next().unwrap_or(' ');
                    stripped_code.push(ch);
                    if b == b'\\' && i + 1 < bytes.len() {
                        i += 1;
                        let ch2 = raw_line[i..].chars().next().unwrap_or(' ');
                        stripped_code.push(ch2);
                        i += ch2.len_utf8();
                        continue;
                    }
                    if b == q { q = 0; }
                    i += ch.len_utf8();
                    continue;
                }
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                    break;
                }
                if i + 1 < bytes.len() && bytes[i] == b'(' && bytes[i + 1] == b'*' && bytes.get(i + 2) != Some(&b')') {
                    in_bc2 = true;
                    i += 2;
                    continue;
                }
                let b = bytes[i];
                if b == b'"' {
                    q = b;
                } else if b == b'(' || b == b'[' || b == b'{' {
                    pdelta += 1;
                } else if b == b')' || b == b']' || b == b'}' {
                    pdelta -= 1;
                }
                let ch = raw_line[i..].chars().next().unwrap_or(' ');
                stripped_code.push(ch);
                i += ch.len_utf8();
            }
            let code = stripped_code.trim();
            if code.is_empty() {
                continue;
            }
            per_line_code[ln] = code.to_string();
            per_line_pdelta[ln] = pdelta;

            let cb = code.as_bytes();
            let mut toks: Vec<String> = Vec::new();
            let mut j = 0usize;
            while j < cb.len() {
                if cb[j].is_ascii_digit() {
                    while j < cb.len() && (cb[j].is_ascii_alphanumeric() || cb[j] == b'_' || cb[j] == b'.') {
                        j += 1;
                    }
                    toks.push("0".to_string());
                    continue;
                }
                if cb[j] == b'"' {
                    j += 1;
                    while j < cb.len() && cb[j] != b'"' {
                        if cb[j] == b'\\' && j + 1 < cb.len() { j += 2; continue; }
                        j += 1;
                    }
                    if j < cb.len() { j += 1; }
                    toks.push("0".to_string());
                    continue;
                }
                if is_id_start(cb[j]) {
                    let st = j;
                    j += 1;
                    while j < cb.len() {
                        if is_id_cont(cb[j]) { j += 1; }
                        else if cb[j] == b'.' && j + 1 < cb.len() && is_id_start(cb[j + 1]) { j += 1; }
                        else { break; }
                    }
                    let raw_norm = &code[st..j];
                    let norm_owned = raw_norm.replace("this.", "self.");
                    let norm_owned = if norm_owned == "this" { "self".to_string() } else { norm_owned };
                    let norm = norm_owned.as_str();
                    let mut kf = j;
                    while kf < cb.len() && (cb[kf] == b' ' || cb[kf] == b'\t') { kf += 1; }
                    if kf < cb.len() && cb[kf] == b'=' && (kf + 1 >= cb.len() || cb[kf + 1] != b'=') {
                        let mut k_out = r#"["k","#.to_string();
                        push_string(&mut k_out, norm);
                        k_out.push_str(",0]");
                        toks.push(k_out);
                    }
                    let is_c = if kf < cb.len() {
                        cb[kf] == b'('
                            || (kf > j && (is_id_start(cb[kf]) || cb[kf].is_ascii_digit() || matches!(cb[kf], b'"' | b'[' | b'{')))
                    } else {
                        false
                    };
                    if is_c {
                        let recv = if let Some((h, _)) = norm.rsplit_once('.') {
                            let mut r = r#"["n","#.to_string();
                            push_string(&mut r, h);
                            r.push(']');
                            r
                        } else {
                            "0".to_string()
                        };
                        let args_s = if kf < cb.len() && cb[kf] == b'(' && kf + 1 < cb.len() && cb[kf + 1] == b')' { "[]" } else { "[0]" };
                        toks.push(format!(r#"["c",0,{recv},{args_s}]"#));
                    }
                    let mut n_out = r#"["n","#.to_string();
                    push_string(&mut n_out, norm);
                    n_out.push(']');
                    toks.push(n_out);
                    if raw_norm != norm {
                        let mut nr = r#"["n","#.to_string();
                        push_string(&mut nr, raw_norm);
                        nr.push(']');
                        toks.push(nr);
                    }
                    if let Some((head, last_seg)) = norm.rsplit_once('.') {
                        if !head.is_empty() {
                            let mut nh = r#"["n","#.to_string();
                            push_string(&mut nh, head);
                            nh.push(']');
                            toks.push(nh);
                        }
                        if !last_seg.is_empty() {
                            let mut n2 = r#"["n","#.to_string();
                            push_string(&mut n2, last_seg);
                            n2.push(']');
                            toks.push(n2);
                        }
                    }
                    continue;
                }
                j += 1;
            }
            per_line_tokens[ln] = toks;
        }

        let is_safe_callee = |path: &str| -> bool {
            if path.is_empty() || path.starts_with('.') || path.ends_with('.') || path.contains("().") {
                return false;
            }
            !matches!(
                path,
                "assert" | "return" | "yield" | "await" | "throw" | "raise" | "del" | "delete"
                    | "new" | "sizeof" | "defined" | "if" | "elif" | "else" | "for" | "foreach"
                    | "while" | "until" | "unless" | "switch" | "try" | "except" | "finally"
                    | "with" | "do" | "match" | "let" | "in" | "then" | "and" | "or" | "not"
            )
        };
        let safe_call_idx: Vec<usize> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().position(|c| is_safe_callee(&c.path)).unwrap_or(0))
            .collect();
        let mut extra_events: Vec<Vec<String>> = vec![Vec::new(); n_syms];

        for ln in 1..=n_lines {
            let code = per_line_code[ln].as_str();
            if code.is_empty() {
                continue;
            }
            let ln32 = ln as u32;
            let mut inner_sym = 0usize;
            let mut orig_inner_sym = 0usize;
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en {
                    inner_sym = si;
                    if si < orig_n_syms { orig_inner_sym = si; }
                }
            }
            let mut target_syms: Vec<usize> = vec![0];
            if orig_inner_sym != 0 && !target_syms.contains(&orig_inner_sym) { target_syms.push(orig_inner_sym); }
            if inner_sym != 0 && !target_syms.contains(&inner_sym) { target_syms.push(inner_sym); }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) {
                    target_syms.push(last_si);
                }
            }

            let is_big_assign = code.contains('=')
                && (per_line_pdelta[ln] > 0 || code.ends_with('=') || (ln < n_lines && per_line_pdelta[ln + 1] > 0));
            let (z_cap, c_cap, max_span) = if is_big_assign {
                (4096usize, 1024usize, 3000usize)
            } else {
                (256usize, 128usize, 60usize)
            };
            let mut vals: Vec<String> = vec!["0".to_string()];
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut zc = 1usize;
            let mut cc = 0usize;
            let mut pdepth = per_line_pdelta[ln];
            for t in &per_line_tokens[ln] {
                if t == "0" { if zc < z_cap { vals.push("0".to_string()); zc += 1; } }
                else if t.starts_with(r#"["c","#) { if cc < c_cap { vals.push(t.clone()); cc += 1; } }
                else if seen.insert(t.clone()) { vals.push(t.clone()); }
            }
            let mut cur = ln + 1;
            let max_cur = (ln + max_span).min(n_lines);
            while cur <= max_cur {
                if pdepth <= 0 && cur > ln + 4 && !code.ends_with('=') { break; }
                if pdepth <= 0 && cur > ln + 30 { break; }
                for t in &per_line_tokens[cur] {
                    if t == "0" { if zc < z_cap { vals.push("0".to_string()); zc += 1; } }
                    else if t.starts_with(r#"["c","#) { if cc < c_cap { vals.push(t.clone()); cc += 1; } }
                    else if seen.insert(t.clone()) { vals.push(t.clone()); }
                }
                pdepth += per_line_pdelta[cur];
                cur += 1;
            }
            let mut val_json = String::from(r#"["m",["#);
            for (ti, tok) in vals.iter().enumerate() {
                if ti > 0 { val_json.push(','); }
                val_json.push_str(tok);
            }
            val_json.push_str("]]");

            // Assignment `a` for `let x = ...`, `let a, b = ...`, `x <- ...`, and multiline paren-assign
            let mut cand_targets: Vec<Vec<String>> = Vec::new();
            let cb_eq = code.as_bytes();
            let mut eq_top: Option<usize> = None;
            let mut bd = 0i32;
            let mut qc: u8 = 0;
            for ci in 0..cb_eq.len() {
                let b = cb_eq[ci];
                if qc != 0 {
                    if b == b'\\' { continue; }
                    if b == qc { qc = 0; }
                    continue;
                }
                if b == b'"' { qc = b; continue; }
                if matches!(b, b'(' | b'[' | b'{') { bd += 1; continue; }
                if matches!(b, b')' | b']' | b'}') { if bd > 0 { bd -= 1; } continue; }
                if bd == 0 && b == b'<' && ci + 1 < cb_eq.len() && cb_eq[ci + 1] == b'-' {
                    eq_top = Some(ci);
                    break;
                }
                if bd == 0 && b == b'=' {
                    let prev = if ci > 0 { cb_eq[ci - 1] } else { 0 };
                    let next = if ci + 1 < cb_eq.len() { cb_eq[ci + 1] } else { 0 };
                    if next != b'=' && next != b'>' && !matches!(prev, b'=' | b'!' | b'<' | b'>' | b':') {
                        eq_top = Some(ci);
                        break;
                    }
                }
            }
            if let Some(ep) = eq_top.or_else(|| code.find("<-")).or_else(|| code.find('=')) {
                let lhs_part = code[..ep].trim();
                let lhs_raw = lhs_part.split_whitespace().last().unwrap_or("").trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '.');
                let lhs_owned = lhs_raw.replace("this.", "self.");
                let lhs_nm = lhs_owned.as_str();
                if !lhs_nm.is_empty() && is_id_start(lhs_nm.as_bytes()[0]) {
                    cand_targets.push(vec![lhs_nm.to_string()]);
                    if lhs_raw != lhs_nm && !lhs_raw.is_empty() {
                        cand_targets.push(vec![lhs_raw.to_string()]);
                    }
                }
                if let Some((before_col, _)) = lhs_part.split_once(':') {
                    let bc_nm = before_col.split_whitespace().last().unwrap_or("").trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '.');
                    if !bc_nm.is_empty() && is_id_start(bc_nm.as_bytes()[0]) && bc_nm != lhs_nm {
                        cand_targets.push(vec![bc_nm.to_string()]);
                    }
                }
                if lhs_part.contains(',') {
                    let mut tuple_t: Vec<String> = Vec::new();
                    for seg in lhs_part.split(',') {
                        let w = seg.split_whitespace().last().unwrap_or("").trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
                        if !w.is_empty() && is_id_start(w.as_bytes()[0]) {
                            tuple_t.push(w.to_string());
                        }
                    }
                    if tuple_t.len() >= 2 {
                        cand_targets.push(tuple_t);
                    }
                }
            } else if per_line_pdelta[ln] > 0 {
                // Multiline signature
                let mut combined = String::from(code);
                let mut pd = per_line_pdelta[ln];
                let mut found_eq = false;
                for mcur in (ln + 1)..=(ln + 60).min(n_lines) {
                    let mc = per_line_code[mcur].as_str();
                    if mc.is_empty() { continue; }
                    combined.push(' ');
                    combined.push_str(mc);
                    pd += per_line_pdelta[mcur];
                    if pd <= 0 {
                        if mc.contains('=') { found_eq = true; }
                        break;
                    }
                }
                if found_eq {
                    if let Some(ep) = combined.rfind('=') {
                        let lhs_m = combined[..ep].trim();
                        if lhs_m.contains(',') {
                            let mut ml_last: Vec<String> = Vec::new();
                            for seg in lhs_m.split(',') {
                                let w = seg.split_whitespace().last().unwrap_or("").trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
                                if !w.is_empty() && is_id_start(w.as_bytes()[0]) {
                                    ml_last.push(w.to_string());
                                }
                            }
                            if ml_last.len() >= 2 {
                                cand_targets.push(ml_last);
                            }
                        }
                    }
                }
            }
            for tlist in cand_targets {
                let mut emit_lns: Vec<u32> = vec![ln32];
                if ln > 2 && per_line_code[ln - 1].is_empty() && (code.starts_with("let ") || code.starts_with("use ")) {
                    emit_lns.push((ln - 2) as u32);
                }
                for eln in emit_lns {
                    let mut ev = format!(r#"["a",{eln},["#);
                    for (ti, t) in tlist.iter().enumerate() {
                        if ti > 0 { ev.push(','); }
                        push_string(&mut ev, t);
                    }
                    ev.push_str("],");
                    ev.push_str(&val_json);
                    ev.push_str(",0]");
                    for &tsi in &target_syms {
                        let sidx = safe_call_idx[tsi];
                        if sidx == 0 {
                            extra_events[tsi].push(ev.clone());
                        } else {
                            extra_events[tsi].push(ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                        }
                    }
                }
            }
            // Return `r` (F# has implicit returns on `let x = expr` as well as `return`/`yield`)
            let ev_r = format!(r#"["r",{ln32},{val_json}]"#);
            let ev_x = format!(r#"["x",{ln32},{val_json},0]"#);
            for &tsi in &target_syms {
                let sidx = safe_call_idx[tsi];
                if sidx == 0 {
                    extra_events[tsi].push(ev_r.clone());
                    extra_events[tsi].push(ev_x.clone());
                } else {
                    let rep = format!(r#"["c",{sidx},"#);
                    extra_events[tsi].push(ev_r.replace(r#"["c",0,"#, &rep));
                    extra_events[tsi].push(ev_x.replace(r#"["c",0,"#, &rep));
                }
            }
        }

        for (si, evs) in extra_events.into_iter().enumerate() {
            if evs.is_empty() { continue; }
            let sym = &mut self.facts.symbols[si];
            for ev in evs {
                if !sym.events.is_empty() { sym.events.push(','); }
                sym.events.push_str(&ev);
            }
        }
    }

    #[allow(dead_code)]
    fn augment_zig_l1(&mut self) {
        let src_str = std::str::from_utf8(self.src).unwrap_or("");
        let lines: Vec<&str> = src_str.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() {
            return;
        }
        let orig_n_syms = self.facts.symbols.len();
        let is_id_start = |b: u8| b.is_ascii_alphabetic() || b == b'_' || b == b'@';
        let is_id_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

        // 1. Missing Zig `fn name(` symbols (with unique keys)
        let mut existing_syms: std::collections::HashSet<(String, u32)> = self
            .facts
            .symbols
            .iter()
            .map(|s| (s.name.clone(), s.line))
            .collect();
        let mut existing_keys: std::collections::HashSet<String> = self
            .facts
            .symbols
            .iter()
            .map(|s| s.key.clone())
            .collect();
        for (idx, raw_line) in lines.iter().enumerate() {
            let ln32 = (idx + 1) as u32;
            let tr = raw_line.trim();
            if tr.is_empty() || tr.starts_with("//") || tr.starts_with("\\") {
                continue;
            }
            let code_no_c = tr.split("//").next().unwrap_or(tr).trim();
            if let Some(fpos) = code_no_c.find("fn ") {
                if fpos == 0 || !is_id_cont(code_no_c.as_bytes()[fpos - 1]) {
                    let rest = code_no_c[fpos + 3..].trim_start();
                    let nm: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    if !nm.is_empty() && nm.as_bytes()[0].is_ascii_alphabetic() && existing_syms.insert((nm.clone(), ln32)) {
                        let key = if existing_keys.insert(nm.clone()) {
                            nm.clone()
                        } else {
                            let k = format!("{nm}@{ln32}");
                            existing_keys.insert(k.clone());
                            k
                        };
                        self.facts.symbols.push(Symbol {
                            key, name: nm, owner: String::new(), kind: KIND_FUNCTION,
                            line: ln32, end_line: n_lines as u32, bases: Vec::new(), calls: Vec::new(),
                            events: String::new(), ..Symbol::default()
                        });
                    }
                }
            }
        }

        // 2. General Zig call scanner (`@builtin(`, `ident(`, `a.b(`, `enum(`) + multiset `var` bindings
        let n_syms = self.facts.symbols.len();
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().map(|c| (c.path.clone(), c.line)).collect())
            .collect();
        let mut sym_spans: Vec<(u32, u32, usize)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, s)| (s.line, s.end_line.max(s.line), i))
            .collect();
        sym_spans.sort_by_key(|&(st, _, _)| st);

        for (idx, raw_line) in lines.iter().enumerate() {
            let ln32 = (idx + 1) as u32;
            let tr = raw_line.trim();
            if tr.is_empty() || tr.starts_with("//") || tr.starts_with("\\") {
                continue;
            }
            let mut inner_sym = 0usize;
            let mut orig_inner_sym = 0usize;
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en {
                    inner_sym = si;
                    if si < orig_n_syms { orig_inner_sym = si; }
                }
            }
            let mut target_syms: Vec<usize> = vec![0];
            if orig_inner_sym != 0 && !target_syms.contains(&orig_inner_sym) { target_syms.push(orig_inner_sym); }
            if inner_sym != 0 && !target_syms.contains(&inner_sym) { target_syms.push(inner_sym); }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) { target_syms.push(last_si); }
            }

            let bytes = raw_line.as_bytes();
            let mut i = 0usize;
            while i < bytes.len() {
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' { break; }
                if bytes[i] == b'"' || bytes[i] == b'\'' {
                    let q = bytes[i];
                    i += 1;
                    while i < bytes.len() && bytes[i] != q {
                        if bytes[i] == b'\\' && i + 1 < bytes.len() { i += 2; continue; }
                        i += 1;
                    }
                    if i < bytes.len() { i += 1; }
                    continue;
                }
                if is_id_start(bytes[i]) {
                    let st = i;
                    i += 1;
                    while i < bytes.len() {
                        if is_id_cont(bytes[i]) { i += 1; }
                        else if bytes[i] == b'.' && i + 1 < bytes.len() && is_id_start(bytes[i + 1]) { i += 1; }
                        else { break; }
                    }
                    let raw_id = &raw_line[st..i];
                    let mut k = i;
                    while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') { k += 1; }
                    if k < bytes.len() && bytes[k] == b'(' {
                        let norm = raw_id.trim_start_matches('@');
                        if !norm.is_empty() && !matches!(norm, "if" | "while" | "for" | "switch" | "catch" | "return") {
                            let mut cands: Vec<String> = vec![norm.to_string()];
                            let segs: Vec<&str> = norm.split('.').collect();
                            if segs.len() >= 2 {
                                cands.push(format!("{}.{}", segs[segs.len() - 2], segs[segs.len() - 1]));
                                cands.push(segs[segs.len() - 1].to_string());
                            }
                            for cand in cands {
                                if cand.is_empty() { continue; }
                                for &si in &target_syms {
                                    if si < n_syms && existing_calls[si].insert((cand.clone(), ln32)) {
                                        self.facts.symbols[si].calls.push(Call { path: cand.clone(), line: ln32 });
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }
                i += 1;
            }

            // Zig var bindings: `[pub] const|var Name = [mod.]UpperType(...)`
            let code_part = raw_line.split("//").next().unwrap_or(raw_line).trim();
            if let Some(ep) = code_part.find('=') {
                let prev_b = if ep > 0 { code_part.as_bytes()[ep - 1] } else { 0 };
                let next_b = code_part.as_bytes().get(ep + 1).copied().unwrap_or(0);
                if next_b != b'=' && next_b != b'>' && !matches!(prev_b, b'=' | b'!' | b'<' | b'>') {
                    let lhs_s = code_part[..ep].split(':').next().unwrap_or("").trim();
                    let var_nm = lhs_s.split_whitespace().last().unwrap_or("");
                    let rhs_s = code_part[ep + 1..].trim_start();
                    let rhs_id: String = rhs_s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
                    if !var_nm.is_empty() && var_nm.as_bytes()[0].is_ascii_alphabetic() {
                        for seg in [rhs_id.split('.').next().unwrap_or(""), rhs_id.rsplit('.').next().unwrap_or("")] {
                            if !seg.is_empty() && seg.as_bytes()[0].is_ascii_uppercase() {
                                self.facts.vars.push((var_nm.to_string(), seg.to_string()));
                            }
                        }
                    }
                }
            }
        }

        // 3. General Zig a/r/x event augmentation pass
        let mut per_line_code: Vec<String> = vec![String::new(); n_lines + 1];
        let mut per_line_pdelta: Vec<i32> = vec![0; n_lines + 1];
        let mut per_line_tokens: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];
        for (idx, raw_line) in lines.iter().enumerate() {
            let ln = idx + 1;
            let tr = raw_line.trim();
            if tr.is_empty() || tr.starts_with("//") {
                continue;
            }
            if tr.starts_with("\\") {
                per_line_code[ln] = "0".to_string();
                per_line_tokens[ln] = vec!["0".to_string()];
                continue;
            }
            let bytes = raw_line.as_bytes();
            let mut stripped_code = String::with_capacity(raw_line.len());
            let mut i = 0usize;
            let mut q: u8 = 0;
            let mut pdelta = 0i32;
            while i < bytes.len() {
                if q != 0 {
                    let b = bytes[i];
                    let ch = raw_line[i..].chars().next().unwrap_or(' ');
                    stripped_code.push(ch);
                    if b == b'\\' && i + 1 < bytes.len() {
                        i += 1;
                        let ch2 = raw_line[i..].chars().next().unwrap_or(' ');
                        stripped_code.push(ch2);
                        i += ch2.len_utf8();
                        continue;
                    }
                    if b == q { q = 0; }
                    i += ch.len_utf8();
                    continue;
                }
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                    break;
                }
                let b = bytes[i];
                if b == b'"' || b == b'\'' {
                    q = b;
                } else if b == b'(' || b == b'[' || b == b'{' {
                    pdelta += 1;
                } else if b == b')' || b == b']' || b == b'}' {
                    pdelta -= 1;
                }
                let ch = raw_line[i..].chars().next().unwrap_or(' ');
                stripped_code.push(ch);
                i += ch.len_utf8();
            }
            let code = stripped_code.trim();
            if code.is_empty() {
                continue;
            }
            per_line_code[ln] = code.to_string();
            per_line_pdelta[ln] = pdelta;

            let cb = code.as_bytes();
            let mut toks: Vec<String> = Vec::new();
            let mut j = 0usize;
            while j < cb.len() {
                if cb[j].is_ascii_digit() {
                    while j < cb.len() && (cb[j].is_ascii_alphanumeric() || cb[j] == b'_' || cb[j] == b'.') {
                        j += 1;
                    }
                    toks.push("0".to_string());
                    continue;
                }
                if cb[j] == b'"' || cb[j] == b'\'' {
                    let qch = cb[j];
                    j += 1;
                    while j < cb.len() && cb[j] != qch {
                        if cb[j] == b'\\' && j + 1 < cb.len() { j += 2; continue; }
                        j += 1;
                    }
                    if j < cb.len() { j += 1; }
                    toks.push("0".to_string());
                    continue;
                }
                if is_id_start(cb[j]) {
                    let st = j;
                    j += 1;
                    while j < cb.len() {
                        if is_id_cont(cb[j]) { j += 1; }
                        else if cb[j] == b'.' && j + 1 < cb.len() && is_id_start(cb[j + 1]) { j += 1; }
                        else { break; }
                    }
                    let raw_id = &code[st..j];
                    let norm = raw_id.trim_start_matches('@');
                    if norm.is_empty() { continue; }
                    let mut kf = j;
                    while kf < cb.len() && (cb[kf] == b' ' || cb[kf] == b'\t') { kf += 1; }
                    if kf < cb.len() && cb[kf] == b'(' {
                        let recv = if let Some((h, _)) = norm.rsplit_once('.') {
                            let mut r = r#"["n","#.to_string();
                            push_string(&mut r, h);
                            r.push(']');
                            r
                        } else {
                            "0".to_string()
                        };
                        let args_s = if kf + 1 < cb.len() && cb[kf + 1] == b')' { "[]" } else { "[0]" };
                        toks.push(format!(r#"["c",0,{recv},{args_s}]"#));
                    }
                    let mut n_out = r#"["n","#.to_string();
                    push_string(&mut n_out, norm);
                    n_out.push(']');
                    toks.push(n_out);
                    if let Some((head, last_seg)) = norm.rsplit_once('.') {
                        if !head.is_empty() {
                            let mut nh = r#"["n","#.to_string();
                            push_string(&mut nh, head);
                            nh.push(']');
                            toks.push(nh);
                        }
                        if !last_seg.is_empty() {
                            let mut n2 = r#"["n","#.to_string();
                            push_string(&mut n2, last_seg);
                            n2.push(']');
                            toks.push(n2);
                        }
                    }
                    continue;
                }
                j += 1;
            }
            per_line_tokens[ln] = toks;
        }

        let is_safe_callee = |path: &str| -> bool {
            if path.is_empty() || path.starts_with('.') || path.ends_with('.') || path.contains("().") {
                return false;
            }
            !matches!(
                path,
                "assert" | "return" | "yield" | "await" | "throw" | "raise" | "del" | "delete"
                    | "new" | "sizeof" | "defined" | "if" | "elif" | "else" | "for" | "foreach"
                    | "while" | "until" | "unless" | "switch" | "try" | "catch" | "finally"
                    | "with" | "do" | "match"
            )
        };
        let safe_call_idx: Vec<usize> = self
            .facts
            .symbols
            .iter()
            .map(|sym| {
                sym.calls
                    .iter()
                    .position(|c| is_safe_callee(&c.path) && c.line != sym.line && c.path != sym.name)
                    .or_else(|| sym.calls.iter().position(|c| is_safe_callee(&c.path)))
                    .unwrap_or(0)
            })
            .collect();
        let mut extra_events: Vec<Vec<String>> = vec![Vec::new(); n_syms];

        for ln in 1..=n_lines {
            let code = per_line_code[ln].as_str();
            if code.is_empty() {
                continue;
            }
            let ln32 = ln as u32;
            let mut inner_sym = 0usize;
            let mut orig_inner_sym = 0usize;
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en {
                    inner_sym = si;
                    if si < orig_n_syms { orig_inner_sym = si; }
                }
            }
            let mut target_syms: Vec<usize> = vec![0];
            if orig_inner_sym != 0 && !target_syms.contains(&orig_inner_sym) { target_syms.push(orig_inner_sym); }
            if inner_sym != 0 && !target_syms.contains(&inner_sym) { target_syms.push(inner_sym); }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) { target_syms.push(last_si); }
            }

            let is_enum_assign = code.contains('=') && code.ends_with(',') && per_line_pdelta[ln] == 0;
            let is_big_assign = code.contains('=')
                && (per_line_pdelta[ln] > 0 || (ln < n_lines && per_line_pdelta[ln + 1] > 0));
            let (z_cap, c_cap, max_span) = if is_big_assign {
                (131072usize, 8192usize, 25000usize)
            } else if is_enum_assign {
                (4096usize, 512usize, 4000usize)
            } else {
                (256usize, 128usize, 60usize)
            };
            let mut vals: Vec<String> = vec!["0".to_string()];
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut zc = 1usize;
            let mut cc = 0usize;
            let mut pdepth = per_line_pdelta[ln];
            for t in &per_line_tokens[ln] {
                if t == "0" { if zc < z_cap { vals.push("0".to_string()); zc += 1; } }
                else if t.starts_with(r#"["c","#) { if cc < c_cap { vals.push(t.clone()); cc += 1; } }
                else if seen.insert(t.clone()) { vals.push(t.clone()); }
            }
            let mut cur = ln + 1;
            let max_cur = (ln + max_span).min(n_lines);
            while cur <= max_cur {
                let prev_c = per_line_code[cur - 1].as_str();
                if pdepth <= 0 && (prev_c.ends_with(';') || prev_c.ends_with('}') || prev_c == "};") {
                    break;
                }
                if !is_enum_assign && pdepth <= 0 && cur > ln + 4 {
                    break;
                }
                for t in &per_line_tokens[cur] {
                    if t == "0" { if zc < z_cap { vals.push("0".to_string()); zc += 1; } }
                    else if t.starts_with(r#"["c","#) { if cc < c_cap { vals.push(t.clone()); cc += 1; } }
                    else if seen.insert(t.clone()) { vals.push(t.clone()); }
                }
                pdepth += per_line_pdelta[cur];
                cur += 1;
            }
            let mut val_json = String::from(r#"["m",["#);
            for (ti, tok) in vals.iter().enumerate() {
                if ti > 0 { val_json.push(','); }
                val_json.push_str(tok);
            }
            val_json.push_str("]]");

            let cb_a = code.as_bytes();
            let mut eq_pos: Option<usize> = None;
            let mut qch: u8 = 0;
            let mut bd = 0i32;
            for ci in 0..cb_a.len() {
                let b = cb_a[ci];
                if qch != 0 {
                    if b == b'\\' { continue; }
                    if b == qch { qch = 0; }
                    continue;
                }
                if b == b'"' || b == b'\'' { qch = b; continue; }
                if matches!(b, b'(' | b'[' | b'{') { bd += 1; continue; }
                if matches!(b, b')' | b']' | b'}') { if bd > 0 { bd -= 1; } continue; }
                if bd == 0 && b == b'=' {
                    let prev = if ci > 0 { cb_a[ci - 1] } else { 0 };
                    let next = if ci + 1 < cb_a.len() { cb_a[ci + 1] } else { 0 };
                    if next != b'=' && next != b'>' && !matches!(prev, b'=' | b'!' | b'<' | b'>') {
                        eq_pos = Some(ci);
                        break;
                    }
                }
            }
            if let Some(ep) = eq_pos {
                let lhs_full = code[..ep].trim_end().trim_end_matches(|c: char| matches!(c, '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '~')).trim_end();
                let lhs_before_col = lhs_full.split(':').next().unwrap_or(lhs_full).trim_end();
                let mut cand_targets: Vec<String> = Vec::new();
                for lp in [lhs_before_col, lhs_full] {
                    let lb = lp.as_bytes();
                    let mut li = lb.len();
                    while li > 0 && (is_id_cont(lb[li - 1]) || lb[li - 1] == b'.') { li -= 1; }
                    let nm = lp[li..].trim_matches('.');
                    if !nm.is_empty() && is_id_start(nm.as_bytes()[0]) && !cand_targets.iter().any(|x| x == nm) {
                        cand_targets.push(nm.to_string());
                    }
                }
                for nm in cand_targets {
                    let mut ev = format!(r#"["a",{ln32},["#);
                    push_string(&mut ev, &nm);
                    ev.push_str("],");
                    ev.push_str(&val_json);
                    ev.push_str(",0]");
                    for &tsi in &target_syms {
                        let sidx = safe_call_idx[tsi];
                        if sidx == 0 {
                            extra_events[tsi].push(ev.clone());
                        } else {
                            extra_events[tsi].push(ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                        }
                    }
                }
            } else {
                let decl_rest = code.strip_prefix("pub ").unwrap_or(code);
                let decl_rest = decl_rest.strip_prefix("const ").or_else(|| decl_rest.strip_prefix("var "));
                if let Some(dr) = decl_rest {
                    let nm: String = dr.trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    if !nm.is_empty() && is_id_start(nm.as_bytes()[0]) {
                        let mut ev = format!(r#"["a",{ln32},["#);
                        push_string(&mut ev, &nm);
                        ev.push_str("],");
                        ev.push_str(&val_json);
                        ev.push_str(",0]");
                        for &tsi in &target_syms {
                            let sidx = safe_call_idx[tsi];
                            if sidx == 0 {
                                extra_events[tsi].push(ev.clone());
                            } else {
                                extra_events[tsi].push(ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                            }
                        }
                    }
                }
            }
            if let Some(rest_for) = code.strip_prefix("for (").or_else(|| code.strip_prefix("for(")) {
                let iter_nm: String = rest_for.trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
                if !iter_nm.is_empty() && is_id_start(iter_nm.as_bytes()[0]) {
                    let mut ev = format!(r#"["a",{ln32},["#);
                    push_string(&mut ev, &iter_nm);
                    ev.push_str("],0,0]");
                    for &tsi in &target_syms {
                        extra_events[tsi].push(ev.clone());
                    }
                }
            }
            if code.starts_with("return ") || code.starts_with("return;") || code.contains(" return ") {
                let ev_r = format!(r#"["r",{ln32},{val_json}]"#);
                for &tsi in &target_syms {
                    let sidx = safe_call_idx[tsi];
                    if sidx == 0 {
                        extra_events[tsi].push(ev_r.clone());
                    } else {
                        extra_events[tsi].push(ev_r.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                    }
                }
            }
            let ev_x = format!(r#"["x",{ln32},{val_json},0]"#);
            for &tsi in &target_syms {
                let sidx = safe_call_idx[tsi];
                if sidx == 0 {
                    extra_events[tsi].push(ev_x.clone());
                } else {
                    extra_events[tsi].push(ev_x.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                }
            }
        }

        for (si, evs) in extra_events.into_iter().enumerate() {
            if evs.is_empty() { continue; }
            let sym = &mut self.facts.symbols[si];
            for ev in evs {
                if !sym.events.is_empty() { sym.events.push(','); }
                sym.events.push_str(&ev);
            }
        }
    }

    fn augment_js(&mut self) {
        let src_str = std::str::from_utf8(self.src).unwrap_or("");
        let lines: Vec<&str> = src_str.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() {
            return;
        }
        let orig_n_syms = self.facts.symbols.len();
        let is_id_start = |b: u8| b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b == b'#' || b == b'@';
        let is_id_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';

        // 1. Missing CommonJS `require('mod')` imports (plain, alias, destructured)
        let mut existing_imports: std::collections::HashSet<(String, String, String)> = self
            .facts
            .imports
            .iter()
            .map(|imp| (imp.module.clone(), imp.name.clone(), imp.alias.clone()))
            .collect();
        let n_syms = self.facts.symbols.len();
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().map(|c| (c.path.clone(), c.line)).collect())
            .collect();
        let mut sym_spans: Vec<(u32, u32, usize)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, s)| (s.line, s.end_line.max(s.line), i))
            .collect();
        sym_spans.sort_by_key(|&(st, _, _)| st);

        let mut in_bc = false;
        for (idx, raw_line) in lines.iter().enumerate() {
            let ln32 = (idx + 1) as u32;
            let bytes = raw_line.as_bytes();
            let mut inner_sym = 0usize;
            let mut orig_inner_sym = 0usize;
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en {
                    inner_sym = si;
                    if si < orig_n_syms { orig_inner_sym = si; }
                }
            }
            let mut target_syms: Vec<usize> = vec![0];
            if orig_inner_sym != 0 && !target_syms.contains(&orig_inner_sym) { target_syms.push(orig_inner_sym); }
            if inner_sym != 0 && !target_syms.contains(&inner_sym) { target_syms.push(inner_sym); }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) { target_syms.push(last_si); }
            }

            // Scan line for block comments, calls, require(), and var bindings
            let mut i = 0usize;
            let mut q: u8 = 0;
            let mut code_buf = String::with_capacity(raw_line.len());
            while i < bytes.len() {
                if in_bc {
                    if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' { in_bc = false; i += 2; }
                    else { i += 1; }
                    continue;
                }
                if q != 0 {
                    let b = bytes[i];
                    let ch = raw_line[i..].chars().next().unwrap_or(' ');
                    code_buf.push(ch);
                    if b == b'\\' && i + 1 < bytes.len() {
                        i += 1;
                        let ch2 = raw_line[i..].chars().next().unwrap_or(' ');
                        code_buf.push(ch2);
                        i += ch2.len_utf8();
                        continue;
                    }
                    if b == q { q = 0; }
                    i += ch.len_utf8();
                    continue;
                }
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' { break; }
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' { in_bc = true; i += 2; continue; }
                let b = bytes[i];
                if b == b'"' || b == b'\'' || b == b'`' { q = b; }
                let ch = raw_line[i..].chars().next().unwrap_or(' ');
                code_buf.push(ch);
                i += ch.len_utf8();
            }
            let code = code_buf.trim();
            if code.is_empty() { continue; }

            // `require('mod')` / `require("mod")`
            let mut rpos = 0usize;
            while let Some(rel) = code[rpos..].find("require(") {
                let p_req = rpos + rel;
                rpos = p_req + 8;
                let after = code[rpos..].trim_start();
                let qch = after.as_bytes().first().copied().unwrap_or(0);
                if qch == b'\'' || qch == b'"' {
                    if let Some(endq) = after[1..].find(qch as char) {
                        let mod_name = &after[1..1 + endq];
                        if !mod_name.is_empty() {
                            let mut emitted = false;
                            let before = code[..p_req].trim_end();
                            if let Some(eq_p) = before.rfind('=') {
                                let lhs = before[..eq_p].trim();
                                if let (Some(lb), Some(rb)) = (lhs.find('{'), lhs.rfind('}')) {
                                    if lb < rb {
                                        for seg in lhs[lb + 1..rb].split(',') {
                                            let st = seg.trim();
                                            if st.is_empty() { continue; }
                                            let (nm, al) = if let Some((k, v)) = st.split_once(':') {
                                                (k.trim(), v.trim())
                                            } else {
                                                (st, st)
                                            };
                                            if !nm.is_empty() && !al.is_empty() {
                                                if existing_imports.insert((mod_name.to_string(), nm.to_string(), al.to_string())) {
                                                    self.facts.imports.push(Import {
                                                        module: mod_name.to_string(), name: nm.to_string(), alias: al.to_string(),
                                                        kind: 0, line: ln32,
                                                    });
                                                }
                                                emitted = true;
                                            }
                                        }
                                    }
                                } else {
                                    let al = lhs.split_whitespace().last().unwrap_or("").trim_end_matches(';');
                                    if !al.is_empty() && is_id_start(al.as_bytes()[0]) {
                                        if existing_imports.insert((mod_name.to_string(), String::new(), al.to_string())) {
                                            self.facts.imports.push(Import {
                                                module: mod_name.to_string(), name: String::new(), alias: al.to_string(),
                                                kind: 0, line: ln32,
                                            });
                                        }
                                        emitted = true;
                                    }
                                }
                            }
                            if existing_imports.insert((mod_name.to_string(), String::new(), String::new())) {
                                self.facts.imports.push(Import {
                                    module: mod_name.to_string(), name: String::new(), alias: String::new(),
                                    kind: 0, line: ln32,
                                });
                            }
                        }
                    }
                }
            }

            // General JS call scanner (`ident(`, `a.b(`, `a?.b(`)
            let cb = code.as_bytes();
            let mut j = 0usize;
            while j < cb.len() {
                if cb[j] == b'"' || cb[j] == b'\'' || cb[j] == b'`' {
                    let qch = cb[j];
                    j += 1;
                    while j < cb.len() && cb[j] != qch {
                        if cb[j] == b'\\' && j + 1 < cb.len() { j += 2; continue; }
                        j += 1;
                    }
                    if j < cb.len() { j += 1; }
                    continue;
                }
                if is_id_start(cb[j]) {
                    let st = j;
                    j += 1;
                    while j < cb.len() {
                        if is_id_cont(cb[j]) { j += 1; }
                        else if cb[j] == b'.' && j + 1 < cb.len() && is_id_start(cb[j + 1]) { j += 1; }
                        else if cb[j] == b'?' && j + 1 < cb.len() && cb[j + 1] == b'.' && j + 2 < cb.len() && is_id_start(cb[j + 2]) { j += 2; }
                        else { break; }
                    }
                    let raw_id = &code[st..j];
                    let mut k = j;
                    while k < cb.len() && (cb[k] == b' ' || cb[k] == b'\t') { k += 1; }
                    if k < cb.len() && cb[k] == b'<' {
                        let mut d = 1i32;
                        let mut g = k + 1;
                        while g < cb.len() && d > 0 {
                            if cb[g] == b'<' { d += 1; }
                            else if cb[g] == b'>' { d -= 1; }
                            else if matches!(cb[g], b';' | b'{' | b'}') { d = -1; break; }
                            g += 1;
                        }
                        if d == 0 {
                            k = g;
                            while k < cb.len() && (cb[k] == b' ' || cb[k] == b'\t') { k += 1; }
                        }
                    }
                    if k < cb.len() && cb[k] == b'(' {
                        let norm = raw_id.replace("?.", ".").trim_start_matches('@').to_string();
                        if !matches!(norm.as_str(), "if"|"for"|"while"|"switch"|"catch"|"return"|"function"|"typeof"|"delete"|"void"|"new"|"await"|"yield") {
                            let mut cands: Vec<String> = vec![norm.clone()];
                            let segs: Vec<&str> = norm.split('.').collect();
                            if segs.len() >= 2 {
                                cands.push(format!("{}.{}", segs[segs.len() - 2], segs[segs.len() - 1]));
                                cands.push(segs[segs.len() - 1].to_string());
                            }
                            for cand in cands {
                                if cand.is_empty() { continue; }
                                for &si in &target_syms {
                                    if si < n_syms && existing_calls[si].insert((cand.clone(), ln32)) {
                                        self.facts.symbols[si].calls.push(Call { path: cand.clone(), line: ln32 });
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }
                j += 1;
            }

            // General JS multiset `var` bindings (`x = [new] [mod.]UpperIdent(` or `{ a, b } = UpperIdent(`)
            if let Some(ep) = code.find('=') {
                let prev_b = if ep > 0 { code.as_bytes()[ep - 1] } else { 0 };
                let next_b = code.as_bytes().get(ep + 1).copied().unwrap_or(0);
                if next_b != b'=' && next_b != b'>' && !matches!(prev_b, b'=' | b'!' | b'<' | b'>') {
                    let lhs = code[..ep].trim();
                    let rhs = code[ep + 1..].trim_start();
                    let rhs = rhs.strip_prefix("await ").unwrap_or(rhs).trim_start();
                    let rhs = rhs.strip_prefix("new ").unwrap_or(rhs).trim_start();
                    let rhs_id: String = rhs.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$' || *c == '.').collect();
                    let rhs_ty = rhs_id.rsplit('.').next().unwrap_or("");
                    if !rhs_ty.is_empty() && rhs_ty.as_bytes()[0].is_ascii_uppercase() {
                        if let (Some(lb), Some(rb)) = (lhs.find('{'), lhs.rfind('}')) {
                            if lb < rb {
                                for seg in lhs[lb + 1..rb].split(',') {
                                    let v = seg.split(':').last().unwrap_or("").split('=').next().unwrap_or("").trim();
                                    if !v.is_empty() && is_id_start(v.as_bytes()[0]) {
                                        self.facts.vars.push((v.to_string(), rhs_ty.to_string()));
                                    }
                                }
                            }
                        } else {
                            let v = lhs.split_whitespace().last().unwrap_or("");
                            if !v.is_empty() && is_id_start(v.as_bytes()[0]) {
                                self.facts.vars.push((v.to_string(), rhs_ty.to_string()));
                            }
                        }
                    }
                }
            }
        }

        // 2. General JS a/r/x event augmentation pass
        let mut per_line_code: Vec<String> = vec![String::new(); n_lines + 1];
        let mut per_line_pdelta: Vec<i32> = vec![0; n_lines + 1];
        let mut per_line_tokens: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];
        let mut in_bc2 = false;
        for (idx, raw_line) in lines.iter().enumerate() {
            let ln = idx + 1;
            let bytes = raw_line.as_bytes();
            let mut stripped_code = String::with_capacity(raw_line.len());
            let mut i = 0usize;
            let mut q: u8 = 0;
            let mut pdelta = 0i32;
            while i < bytes.len() {
                if in_bc2 {
                    if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' { in_bc2 = false; i += 2; }
                    else { i += 1; }
                    continue;
                }
                if q != 0 {
                    let b = bytes[i];
                    let ch = raw_line[i..].chars().next().unwrap_or(' ');
                    stripped_code.push(ch);
                    if b == b'\\' && i + 1 < bytes.len() {
                        i += 1;
                        let ch2 = raw_line[i..].chars().next().unwrap_or(' ');
                        stripped_code.push(ch2);
                        i += ch2.len_utf8();
                        continue;
                    }
                    if b == q { q = 0; }
                    i += ch.len_utf8();
                    continue;
                }
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' { break; }
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' { in_bc2 = true; i += 2; continue; }
                let b = bytes[i];
                if b == b'"' || b == b'\'' || b == b'`' { q = b; }
                else if matches!(b, b'(' | b'[' | b'{') { pdelta += 1; }
                else if matches!(b, b')' | b']' | b'}') { pdelta -= 1; }
                let ch = raw_line[i..].chars().next().unwrap_or(' ');
                stripped_code.push(ch);
                i += ch.len_utf8();
            }
            let code = stripped_code.trim();
            if code.is_empty() { continue; }
            per_line_code[ln] = code.to_string();
            per_line_pdelta[ln] = pdelta;

            let cb = code.as_bytes();
            let mut toks: Vec<String> = Vec::new();
            let mut j = 0usize;
            while j < cb.len() {
                if cb[j].is_ascii_digit() {
                    while j < cb.len() && (cb[j].is_ascii_alphanumeric() || cb[j] == b'_' || cb[j] == b'.') { j += 1; }
                    toks.push("0".to_string());
                    continue;
                }
                if cb[j] == b'"' || cb[j] == b'\'' || cb[j] == b'`' {
                    let qch = cb[j];
                    j += 1;
                    while j < cb.len() && cb[j] != qch {
                        if cb[j] == b'\\' && j + 1 < cb.len() { j += 2; continue; }
                        j += 1;
                    }
                    if j < cb.len() { j += 1; }
                    toks.push("0".to_string());
                    continue;
                }
                if is_id_start(cb[j]) {
                    let st = j;
                    j += 1;
                    while j < cb.len() {
                        if is_id_cont(cb[j]) { j += 1; }
                        else if cb[j] == b'.' && j + 1 < cb.len() && is_id_start(cb[j + 1]) { j += 1; }
                        else if cb[j] == b'?' && j + 1 < cb.len() && cb[j + 1] == b'.' && j + 2 < cb.len() && is_id_start(cb[j + 2]) { j += 2; }
                        else { break; }
                    }
                    let raw_id = &code[st..j];
                    let norm = raw_id.replace("?.", ".").replace("this.", "self.");
                    let norm = if norm == "this" { "self".to_string() } else { norm };
                    let mut kf = j;
                    while kf < cb.len() && (cb[kf] == b' ' || cb[kf] == b'\t') { kf += 1; }
                    if kf < cb.len() && cb[kf] == b':' && (kf + 1 >= cb.len() || cb[kf + 1] != b':') {
                        let mut k_out = r#"["k","#.to_string();
                        push_string(&mut k_out, &norm);
                        k_out.push_str(",0]");
                        toks.push(k_out);
                    }
                    if kf < cb.len() && cb[kf] == b'(' {
                        let recv = if let Some((h, _)) = norm.rsplit_once('.') {
                            let mut r = r#"["n","#.to_string();
                            push_string(&mut r, h);
                            r.push(']');
                            r
                        } else {
                            "0".to_string()
                        };
                        let args_s = if kf + 1 < cb.len() && cb[kf + 1] == b')' { "[]" } else { "[0]" };
                        toks.push(format!(r#"["c",0,{recv},{args_s}]"#));
                    }
                    let mut n_out = r#"["n","#.to_string();
                    push_string(&mut n_out, &norm);
                    n_out.push(']');
                    toks.push(n_out);
                    if let Some((head, last_seg)) = norm.rsplit_once('.') {
                        if !head.is_empty() {
                            let mut nh = r#"["n","#.to_string();
                            push_string(&mut nh, head);
                            nh.push(']');
                            toks.push(nh);
                        }
                        if !last_seg.is_empty() {
                            let mut n2 = r#"["n","#.to_string();
                            push_string(&mut n2, last_seg);
                            n2.push(']');
                            toks.push(n2);
                        }
                    }
                    continue;
                }
                j += 1;
            }
            per_line_tokens[ln] = toks;
        }

        let is_safe_callee = |path: &str| -> bool {
            if path.is_empty() || path.starts_with('.') || path.ends_with('.') || path.contains("().") {
                return false;
            }
            !matches!(
                path,
                "assert" | "return" | "yield" | "await" | "throw" | "raise" | "del" | "delete"
                    | "new" | "sizeof" | "typeof" | "void" | "if" | "elif" | "else" | "for"
                    | "while" | "switch" | "try" | "catch" | "finally" | "with" | "do" | "function"
            )
        };
        let safe_call_idx: Vec<usize> = self
            .facts
            .symbols
            .iter()
            .map(|sym| {
                sym.calls
                    .iter()
                    .position(|c| is_safe_callee(&c.path) && c.line != sym.line && c.path != sym.name)
                    .or_else(|| sym.calls.iter().position(|c| is_safe_callee(&c.path)))
                    .unwrap_or(0)
            })
            .collect();
        let mut extra_events: Vec<Vec<String>> = vec![Vec::new(); n_syms];

        for ln in 1..=n_lines {
            let code = per_line_code[ln].as_str();
            if code.is_empty() { continue; }
            let ln32 = ln as u32;
            let mut inner_sym = 0usize;
            let mut orig_inner_sym = 0usize;
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en {
                    inner_sym = si;
                    if si < orig_n_syms { orig_inner_sym = si; }
                }
            }
            let mut target_syms: Vec<usize> = vec![0];
            if orig_inner_sym != 0 && !target_syms.contains(&orig_inner_sym) { target_syms.push(orig_inner_sym); }
            if inner_sym != 0 && !target_syms.contains(&inner_sym) { target_syms.push(inner_sym); }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) { target_syms.push(last_si); }
            }

            let mut vals: Vec<String> = vec!["0".to_string()];
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut zc = 1usize;
            let mut cc = 0usize;
            let mut pdepth = per_line_pdelta[ln];
            for t in &per_line_tokens[ln] {
                if t == "0" { if zc < 256 { vals.push("0".to_string()); zc += 1; } }
                else if t.starts_with(r#"["c","#) { if cc < 128 { vals.push(t.clone()); cc += 1; } }
                else if seen.insert(t.clone()) { vals.push(t.clone()); }
            }
            let mut cur = ln + 1;
            let max_cur = (ln + 40).min(n_lines);
            while cur <= max_cur {
                let prev_c = per_line_code[cur - 1].as_str();
                if pdepth <= 0 && (prev_c.ends_with(';') || prev_c.ends_with('}')) { break; }
                if pdepth <= 0 && cur > ln + 4 { break; }
                for t in &per_line_tokens[cur] {
                    if t == "0" { if zc < 256 { vals.push("0".to_string()); zc += 1; } }
                    else if t.starts_with(r#"["c","#) { if cc < 512 { vals.push(t.clone()); cc += 1; } }
                    else if seen.insert(t.clone()) { vals.push(t.clone()); }
                }
                pdepth += per_line_pdelta[cur];
                cur += 1;
            }
            let mut val_json = String::from(r#"["m",["#);
            for (ti, tok) in vals.iter().enumerate() {
                if ti > 0 { val_json.push(','); }
                val_json.push_str(tok);
            }
            val_json.push_str("]]");

            let cb_a = code.as_bytes();
            let mut eq_pos: Option<usize> = None;
            let mut qch: u8 = 0;
            let mut bd = 0i32;
            for ci in 0..cb_a.len() {
                let b = cb_a[ci];
                if qch != 0 {
                    if b == b'\\' { continue; }
                    if b == qch { qch = 0; }
                    continue;
                }
                if b == b'"' || b == b'\'' || b == b'`' { qch = b; continue; }
                if matches!(b, b'(' | b'[' | b'{') { bd += 1; continue; }
                if matches!(b, b')' | b']' | b'}') { if bd > 0 { bd -= 1; } continue; }
                if bd == 0 && b == b'=' {
                    let prev = if ci > 0 { cb_a[ci - 1] } else { 0 };
                    let next = if ci + 1 < cb_a.len() { cb_a[ci + 1] } else { 0 };
                    if next != b'=' && next != b'>' && !matches!(prev, b'=' | b'!' | b'<' | b'>') {
                        eq_pos = Some(ci);
                        break;
                    }
                }
            }
            if let Some(ep) = eq_pos {
                let lhs_full = code[..ep].trim_end().trim_end_matches(|c: char| matches!(c, '?' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '~')).trim_end();
                let mut cand_targets: Vec<Vec<String>> = Vec::new();
                if let Some((lb, rb)) = lhs_full.find('{').zip(lhs_full.rfind('}')).or_else(|| lhs_full.find('[').zip(lhs_full.rfind(']'))) {
                    if lb < rb {
                        let mut dlist: Vec<String> = Vec::new();
                        for seg in lhs_full[lb + 1..rb].split(',') {
                            let w = seg.split(':').last().unwrap_or("").split('=').next().unwrap_or("").trim().trim_start_matches("...");
                            if !w.is_empty() && is_id_start(w.as_bytes()[0]) {
                                dlist.push(w.to_string());
                            }
                        }
                        if !dlist.is_empty() {
                            cand_targets.push(dlist);
                        }
                    }
                }
                let lb = lhs_full.as_bytes();
                let mut li = lb.len();
                while li > 0 && (is_id_cont(lb[li - 1]) || lb[li - 1] == b'.') { li -= 1; }
                let nm = lhs_full[li..].trim_matches('.').replace("this.", "self.");
                if !nm.is_empty() && is_id_start(nm.as_bytes()[0]) {
                    cand_targets.push(vec![nm]);
                }
                for tlist in cand_targets {
                    let mut ev = format!(r#"["a",{ln32},["#);
                    for (ti, t) in tlist.iter().enumerate() {
                        if ti > 0 { ev.push(','); }
                        push_string(&mut ev, t);
                    }
                    ev.push_str("],");
                    ev.push_str(&val_json);
                    ev.push_str(",0]");
                    for &tsi in &target_syms {
                        let sidx = safe_call_idx[tsi];
                        if sidx == 0 {
                            extra_events[tsi].push(ev.clone());
                        } else {
                            extra_events[tsi].push(ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                        }
                    }
                }
            }
            if code.starts_with("return ") || code.starts_with("return;") || code.contains(" return ") || code.contains("=>") {
                let ev_r = format!(r#"["r",{ln32},{val_json}]"#);
                for &tsi in &target_syms {
                    let sidx = safe_call_idx[tsi];
                    if sidx == 0 {
                        extra_events[tsi].push(ev_r.clone());
                    } else {
                        extra_events[tsi].push(ev_r.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                    }
                }
            }
            let ev_x = format!(r#"["x",{ln32},{val_json},0]"#);
            for &tsi in &target_syms {
                let sidx = safe_call_idx[tsi];
                if sidx == 0 {
                    extra_events[tsi].push(ev_x.clone());
                } else {
                    extra_events[tsi].push(ev_x.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                }
            }
        }

        for (si, evs) in extra_events.into_iter().enumerate() {
            if evs.is_empty() { continue; }
            let sym = &mut self.facts.symbols[si];
            for ev in evs {
                if !sym.events.is_empty() { sym.events.push(','); }
                sym.events.push_str(&ev);
            }
        }
    }

    fn augment_python(&mut self) {
        let src_str = std::str::from_utf8(self.src).unwrap_or("");
        let lines: Vec<&str> = src_str.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() {
            return;
        }
        let orig_n_syms = self.facts.symbols.len();
        let n_syms = self.facts.symbols.len();
        let is_id_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
        let is_id_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().map(|c| (c.path.clone(), c.line)).collect())
            .collect();
        let mut sym_spans: Vec<(u32, u32, usize)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, s)| (s.line, s.end_line.max(s.line), i))
            .collect();
        sym_spans.sort_by_key(|&(st, _, _)| st);

        let mut per_line_code: Vec<String> = vec![String::new(); n_lines + 1];
        let mut per_line_pdelta: Vec<i32> = vec![0; n_lines + 1];
        let mut per_line_tokens: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];

        let mut in_triple: u8 = 0;
        for (idx, raw_line) in lines.iter().enumerate() {
            let ln = idx + 1;
            let ln32 = ln as u32;
            let bytes = raw_line.as_bytes();
            let mut stripped_code = String::with_capacity(raw_line.len());
            let mut i = 0usize;
            let mut q: u8 = 0;
            let mut pdelta = 0i32;
            while i < bytes.len() {
                if in_triple != 0 {
                    if i + 2 < bytes.len() && bytes[i] == in_triple && bytes[i + 1] == in_triple && bytes[i + 2] == in_triple {
                        in_triple = 0;
                        stripped_code.push('0');
                        i += 3;
                    } else if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                    } else {
                        i += 1;
                    }
                    continue;
                }
                if q != 0 {
                    let b = bytes[i];
                    let ch = raw_line[i..].chars().next().unwrap_or(' ');
                    stripped_code.push(ch);
                    if b == b'\\' && i + 1 < bytes.len() {
                        i += 1;
                        let ch2 = raw_line[i..].chars().next().unwrap_or(' ');
                        stripped_code.push(ch2);
                        i += ch2.len_utf8();
                        continue;
                    }
                    if b == q { q = 0; }
                    i += ch.len_utf8();
                    continue;
                }
                if bytes[i] == b'#' { break; }
                if i + 2 < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') && bytes[i + 1] == bytes[i] && bytes[i + 2] == bytes[i] {
                    in_triple = bytes[i];
                    stripped_code.push('0');
                    i += 3;
                    continue;
                }
                let b = bytes[i];
                if b == b'"' || b == b'\'' { q = b; }
                else if matches!(b, b'(' | b'[' | b'{') { pdelta += 1; }
                else if matches!(b, b')' | b']' | b'}') { pdelta -= 1; }
                let ch = raw_line[i..].chars().next().unwrap_or(' ');
                stripped_code.push(ch);
                i += ch.len_utf8();
            }
            let code = stripped_code.trim();
            if code.is_empty() { continue; }
            per_line_code[ln] = code.to_string();
            per_line_pdelta[ln] = pdelta;

            let mut inner_sym = 0usize;
            let mut orig_inner_sym = 0usize;
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en {
                    inner_sym = si;
                    if si < orig_n_syms { orig_inner_sym = si; }
                }
            }
            let mut target_syms: Vec<usize> = vec![0];
            if orig_inner_sym != 0 && !target_syms.contains(&orig_inner_sym) { target_syms.push(orig_inner_sym); }
            if inner_sym != 0 && !target_syms.contains(&inner_sym) { target_syms.push(inner_sym); }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) { target_syms.push(last_si); }
            }

            let cb = code.as_bytes();
            let mut toks: Vec<String> = Vec::new();
            let mut j = 0usize;
            while j < cb.len() {
                if cb[j].is_ascii_digit() {
                    while j < cb.len() && (cb[j].is_ascii_alphanumeric() || cb[j] == b'_' || cb[j] == b'.') { j += 1; }
                    toks.push("0".to_string());
                    continue;
                }
                if cb[j] == b'"' || cb[j] == b'\'' {
                    let qch = cb[j];
                    j += 1;
                    while j < cb.len() && cb[j] != qch {
                        if cb[j] == b'\\' && j + 1 < cb.len() { j += 2; continue; }
                        j += 1;
                    }
                    if j < cb.len() { j += 1; }
                    toks.push("0".to_string());
                    continue;
                }
                if is_id_start(cb[j]) {
                    let st = j;
                    j += 1;
                    while j < cb.len() {
                        if is_id_cont(cb[j]) { j += 1; }
                        else if cb[j] == b'.' && j + 1 < cb.len() && is_id_start(cb[j + 1]) { j += 1; }
                        else { break; }
                    }
                    let norm = &code[st..j];
                    let mut kf = j;
                    while kf < cb.len() && (cb[kf] == b' ' || cb[kf] == b'\t') { kf += 1; }
                    if kf < cb.len() && cb[kf] == b'=' && (kf + 1 >= cb.len() || cb[kf + 1] != b'=') {
                        let mut k_out = r#"["k","#.to_string();
                        push_string(&mut k_out, norm);
                        k_out.push_str(",0]");
                        toks.push(k_out);
                    }
                    if kf < cb.len() && cb[kf] == b'(' {
                        if !matches!(norm, "if"|"elif"|"while"|"for"|"return"|"yield"|"assert"|"del"|"not"|"and"|"or"|"in"|"is"|"lambda"|"def"|"class"|"with"|"except"|"raise") {
                            let mut cands: Vec<String> = vec![norm.to_string()];
                            let segs: Vec<&str> = norm.split('.').collect();
                            if segs.len() >= 2 {
                                cands.push(format!("{}.{}", segs[segs.len() - 2], segs[segs.len() - 1]));
                                cands.push(segs[segs.len() - 1].to_string());
                            }
                            for cand in cands {
                                for &si in &target_syms {
                                    if si < n_syms && existing_calls[si].insert((cand.clone(), ln32)) {
                                        self.facts.symbols[si].calls.push(Call { path: cand.clone(), line: ln32 });
                                    }
                                }
                            }
                        }
                        let recv = if let Some((h, _)) = norm.rsplit_once('.') {
                            let mut r = r#"["n","#.to_string();
                            push_string(&mut r, h);
                            r.push(']');
                            r
                        } else {
                            "0".to_string()
                        };
                        let args_s = if kf + 1 < cb.len() && cb[kf + 1] == b')' { "[]" } else { "[0]" };
                        toks.push(format!(r#"["c",0,{recv},{args_s}]"#));
                    }
                    let mut n_out = r#"["n","#.to_string();
                    push_string(&mut n_out, norm);
                    n_out.push(']');
                    toks.push(n_out);
                    if norm == "None" {
                        toks.push(r#"["n","null"]"#.to_string());
                    }
                    if let Some((head, last_seg)) = norm.rsplit_once('.') {
                        if !head.is_empty() {
                            let mut nh = r#"["n","#.to_string();
                            push_string(&mut nh, head);
                            nh.push(']');
                            toks.push(nh);
                        }
                        if !last_seg.is_empty() {
                            let mut n2 = r#"["n","#.to_string();
                            push_string(&mut n2, last_seg);
                            n2.push(']');
                            toks.push(n2);
                        }
                    }
                    continue;
                }
                j += 1;
            }
            per_line_tokens[ln] = toks;
        }

        let is_safe_callee = |path: &str| -> bool {
            if path.is_empty() || path.starts_with('.') || path.ends_with('.') || path.contains("().") {
                return false;
            }
            !matches!(
                path,
                "assert" | "return" | "yield" | "await" | "throw" | "raise" | "del" | "delete"
                    | "new" | "sizeof" | "if" | "elif" | "else" | "for" | "while" | "try"
                    | "except" | "finally" | "with" | "def" | "class" | "lambda" | "not" | "and" | "or"
            )
        };
        let safe_call_idx: Vec<usize> = self
            .facts
            .symbols
            .iter()
            .map(|sym| {
                sym.calls
                    .iter()
                    .position(|c| is_safe_callee(&c.path) && c.line != sym.line && c.path != sym.name)
                    .or_else(|| sym.calls.iter().position(|c| is_safe_callee(&c.path)))
                    .unwrap_or(0)
            })
            .collect();
        let mut extra_events: Vec<Vec<String>> = vec![Vec::new(); n_syms];

        for ln in 1..=n_lines {
            let code = per_line_code[ln].as_str();
            if code.is_empty() { continue; }
            let ln32 = ln as u32;
            let mut inner_sym = 0usize;
            let mut orig_inner_sym = 0usize;
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en {
                    inner_sym = si;
                    if si < orig_n_syms { orig_inner_sym = si; }
                }
            }
            let mut target_syms: Vec<usize> = vec![0];
            if orig_inner_sym != 0 && !target_syms.contains(&orig_inner_sym) { target_syms.push(orig_inner_sym); }
            if inner_sym != 0 && !target_syms.contains(&inner_sym) { target_syms.push(inner_sym); }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) { target_syms.push(last_si); }
            }

            let is_big_assign = code.contains('=')
                && (per_line_pdelta[ln] > 0 || (ln < n_lines && per_line_pdelta[ln + 1] > 0));
            let (z_cap, c_cap, max_span) = if is_big_assign {
                (2048usize, 512usize, 300usize)
            } else {
                (128usize, 64usize, 40usize)
            };
            let mut vals: Vec<String> = vec!["0".to_string()];
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut zc = 1usize;
            let mut cc = 0usize;
            let mut pdepth = per_line_pdelta[ln];
            for t in &per_line_tokens[ln] {
                if t == "0" { if zc < z_cap { vals.push("0".to_string()); zc += 1; } }
                else if t.starts_with(r#"["c","#) { if cc < c_cap { vals.push(t.clone()); cc += 1; } }
                else if seen.insert(t.clone()) { vals.push(t.clone()); }
            }
            let mut cur = ln + 1;
            let max_cur = (ln + max_span).min(n_lines);
            while cur <= max_cur {
                if pdepth <= 0 && cur > ln + 2 { break; }
                for t in &per_line_tokens[cur] {
                    if t == "0" { if zc < z_cap { vals.push("0".to_string()); zc += 1; } }
                    else if t.starts_with(r#"["c","#) { if cc < c_cap { vals.push(t.clone()); cc += 1; } }
                    else if seen.insert(t.clone()) { vals.push(t.clone()); }
                }
                pdepth += per_line_pdelta[cur];
                cur += 1;
            }
            let mut val_json = String::from(r#"["m",["#);
            for (ti, tok) in vals.iter().enumerate() {
                if ti > 0 { val_json.push(','); }
                val_json.push_str(tok);
            }
            val_json.push_str("]]");

            let mut cand_targets: Vec<Vec<String>> = Vec::new();
            let cb_a = code.as_bytes();
            let mut eq_pos: Option<usize> = None;
            let mut qch: u8 = 0;
            let mut bd = 0i32;
            for ci in 0..cb_a.len() {
                let b = cb_a[ci];
                if qch != 0 {
                    if b == b'\\' { continue; }
                    if b == qch { qch = 0; }
                    continue;
                }
                if b == b'"' || b == b'\'' { qch = b; continue; }
                if matches!(b, b'(' | b'[' | b'{') { bd += 1; continue; }
                if matches!(b, b')' | b']' | b'}') { if bd > 0 { bd -= 1; } continue; }
                if bd == 0 && b == b'=' {
                    let prev = if ci > 0 { cb_a[ci - 1] } else { 0 };
                    let next = if ci + 1 < cb_a.len() { cb_a[ci + 1] } else { 0 };
                    if next != b'=' && !matches!(prev, b'=' | b'!' | b'<' | b'>') {
                        eq_pos = Some(ci);
                        break;
                    }
                }
            }
            if let Some(ep) = eq_pos {
                let lhs_full = code[..ep].trim_end().trim_end_matches(|c: char| matches!(c, '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | ':' | '@')).trim_end();
                let lhs_no_ann = lhs_full.split(':').next().unwrap_or(lhs_full).trim();
                if lhs_no_ann.contains(',') {
                    let mut tlist: Vec<String> = Vec::new();
                    for seg in lhs_no_ann.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']')).split(',') {
                        let w = seg.trim().trim_start_matches('*');
                        if !w.is_empty() && is_id_start(w.as_bytes()[0]) {
                            tlist.push(w.to_string());
                        }
                    }
                    if !tlist.is_empty() { cand_targets.push(tlist); }
                }
                let lb = lhs_no_ann.as_bytes();
                let mut li = lb.len();
                while li > 0 && (is_id_cont(lb[li - 1]) || lb[li - 1] == b'.') { li -= 1; }
                let nm = lhs_no_ann[li..].trim_matches('.');
                if !nm.is_empty() && is_id_start(nm.as_bytes()[0]) {
                    cand_targets.push(vec![nm.to_string()]);
                }
            }
            if let Some(as_pos) = code.rfind(" as ") {
                let after_as = code[as_pos + 4..].trim().trim_end_matches(':').trim();
                let inner = after_as.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']'));
                let mut alist: Vec<String> = Vec::new();
                for seg in inner.split(',') {
                    let w = seg.trim();
                    if !w.is_empty() && is_id_start(w.as_bytes()[0]) {
                        alist.push(w.to_string());
                    }
                }
                if !alist.is_empty() { cand_targets.push(alist); }
            }
            if let Some(rest_for) = code.strip_prefix("for ").or_else(|| code.strip_prefix("async for ")) {
                if let Some(in_pos) = rest_for.find(" in ") {
                    let lhs_f = rest_for[..in_pos].trim().trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']'));
                    let mut flist: Vec<String> = Vec::new();
                    for seg in lhs_f.split(',') {
                        let w = seg.trim();
                        if !w.is_empty() && is_id_start(w.as_bytes()[0]) {
                            flist.push(w.to_string());
                        }
                    }
                    if !flist.is_empty() { cand_targets.push(flist); }
                }
            }
            for tlist in cand_targets {
                let mut ev = format!(r#"["a",{ln32},["#);
                for (ti, t) in tlist.iter().enumerate() {
                    if ti > 0 { ev.push(','); }
                    push_string(&mut ev, t);
                }
                ev.push_str("],");
                ev.push_str(&val_json);
                ev.push_str(",0]");
                for &tsi in &target_syms {
                    let sidx = safe_call_idx[tsi];
                    if sidx == 0 {
                        extra_events[tsi].push(ev.clone());
                    } else {
                        extra_events[tsi].push(ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                    }
                }
            }
            if code.starts_with("return ") || code == "return" || code.starts_with("yield ") || code.contains(" return ") {
                let ev_r = format!(r#"["r",{ln32},{val_json}]"#);
                for &tsi in &target_syms {
                    let sidx = safe_call_idx[tsi];
                    if sidx == 0 {
                        extra_events[tsi].push(ev_r.clone());
                    } else {
                        extra_events[tsi].push(ev_r.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                    }
                }
            }
            let ev_x = format!(r#"["x",{ln32},{val_json},0]"#);
            for &tsi in &target_syms {
                let sidx = safe_call_idx[tsi];
                if sidx == 0 {
                    extra_events[tsi].push(ev_x.clone());
                } else {
                    extra_events[tsi].push(ev_x.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                }
            }
        }

        for (si, evs) in extra_events.into_iter().enumerate() {
            if evs.is_empty() { continue; }
            let sym = &mut self.facts.symbols[si];
            for ev in evs {
                if !sym.events.is_empty() { sym.events.push(','); }
                sym.events.push_str(&ev);
            }
        }
    }

    fn augment_d(&mut self) {
        let src_str = std::str::from_utf8(self.src).unwrap_or("");
        let lines: Vec<&str> = src_str.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() {
            return;
        }
        let orig_n_syms = self.facts.symbols.len();
        let is_id_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
        let is_id_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

        // Strip //, /* ... */, and /+ ... +/ comments per line while keeping line alignment
        let mut clean_lines: Vec<String> = Vec::with_capacity(n_lines);
        let mut in_block_c = false;
        let mut nest_plus = 0usize;
        for raw_line in &lines {
            let b = raw_line.as_bytes();
            let mut out = String::with_capacity(b.len());
            let mut i = 0usize;
            let mut in_dq = false;
            let mut in_sq = false;
            while i < b.len() {
                if in_block_c {
                    if i + 1 < b.len() && b[i] == b'*' && b[i + 1] == b'/' {
                        in_block_c = false;
                        i += 2;
                    } else {
                        i += 1;
                    }
                    continue;
                }
                if nest_plus > 0 {
                    if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'+' {
                        nest_plus += 1;
                        i += 2;
                    } else if i + 1 < b.len() && b[i] == b'+' && b[i + 1] == b'/' {
                        nest_plus -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                    continue;
                }
                if in_dq {
                    out.push(b[i] as char);
                    if b[i] == b'\\' && i + 1 < b.len() {
                        out.push(b[i + 1] as char);
                        i += 2;
                        continue;
                    }
                    if b[i] == b'"' { in_dq = false; }
                    i += 1;
                    continue;
                }
                if in_sq {
                    out.push(b[i] as char);
                    if b[i] == b'\\' && i + 1 < b.len() {
                        out.push(b[i + 1] as char);
                        i += 2;
                        continue;
                    }
                    if b[i] == b'\'' { in_sq = false; }
                    i += 1;
                    continue;
                }
                if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
                    break;
                }
                if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
                    in_block_c = true;
                    i += 2;
                    continue;
                }
                let _ = nest_plus;
                if b[i] == b'"' { in_dq = true; }
                else if b[i] == b'\'' { in_sq = true; }
                out.push(b[i] as char);
                i += 1;
            }
            clean_lines.push(out);
        }

        // 1. D imports: `[public|private|protected|package|static] import [alias =] mod.path [: sym1, sym2];`
        let mut existing_imports: std::collections::HashSet<(String, String, String)> = self
            .facts
            .imports
            .iter()
            .map(|imp| (imp.module.clone(), imp.name.clone(), imp.alias.clone()))
            .collect();
        for (idx, cl) in clean_lines.iter().enumerate() {
            let ln32 = (idx + 1) as u32;
            let mut tr = cl.trim();
            if tr.is_empty() { continue; }
            for pref in ["public ", "private ", "protected ", "package ", "static "] {
                while let Some(r) = tr.strip_prefix(pref) {
                    tr = r.trim_start();
                }
            }
            let Some(mut rest) = tr.strip_prefix("import ").or_else(|| tr.strip_prefix("import\t")) else { continue };
            // Gather until ';' (up to 6 lines)
            let mut full = rest.to_string();
            let mut k = idx + 1;
            while !full.contains(';') && k < n_lines && k <= idx + 6 {
                full.push(' ');
                full.push_str(clean_lines[k].trim());
                k += 1;
            }
            rest = full.split(';').next().unwrap_or(&full).trim();
            // Handle selective import `mod.path : a, b = c`
            if let Some((mod_part, sel_part)) = rest.split_once(':') {
                let m = mod_part.trim();
                let (m_alias, m_path) = if let Some((al, mp)) = m.split_once('=') {
                    (Some(al.trim()), mp.trim())
                } else {
                    (None, m)
                };
                if !m_path.is_empty() && is_id_start(m_path.as_bytes()[0]) {
                    let base_al = m_alias.unwrap_or_else(|| m_path.rsplit('.').next().unwrap_or(m_path));
                    if existing_imports.insert((m_path.to_string(), String::new(), base_al.to_string())) {
                        self.facts.imports.push(Import {
                            line: ln32,
                            module: m_path.to_string(),
                            name: String::new(),
                            alias: base_al.to_string(),
                            kind: IMPORT_MODULE,
                            ..Import::default()
                        });
                    }
                    for item in sel_part.split(',') {
                        let it = item.trim();
                        if it.is_empty() { continue; }
                        let (al, sym) = if let Some((a, s)) = it.split_once('=') {
                            (a.trim(), s.trim())
                        } else {
                            (it, it)
                        };
                        if !sym.is_empty() && is_id_start(sym.as_bytes()[0]) {
                            let full_mod = format!("{m_path}.{sym}");
                            if existing_imports.insert((full_mod.clone(), String::new(), al.to_string())) {
                                self.facts.imports.push(Import {
                                    line: ln32,
                                    module: full_mod,
                                    name: String::new(),
                                    alias: al.to_string(),
                                    kind: IMPORT_MODULE,
                                    ..Import::default()
                                });
                            }
                            if existing_imports.insert((m_path.to_string(), sym.to_string(), al.to_string())) {
                                self.facts.imports.push(Import {
                                    line: ln32,
                                    module: m_path.to_string(),
                                    name: sym.to_string(),
                                    alias: al.to_string(),
                                    kind: 0u8,
                                    ..Import::default()
                                });
                            }
                        }
                    }
                }
            } else {
                for item in rest.split(',') {
                    let it = item.trim();
                    if it.is_empty() { continue; }
                    let (al, mp) = if let Some((a, m)) = it.split_once('=') {
                        (a.trim(), m.trim())
                    } else {
                        (it.rsplit('.').next().unwrap_or(it), it)
                    };
                    if !mp.is_empty() && is_id_start(mp.as_bytes()[0]) {
                        if existing_imports.insert((mp.to_string(), String::new(), al.to_string())) {
                            self.facts.imports.push(Import {
                                line: ln32,
                                module: mp.to_string(),
                                name: String::new(),
                                alias: al.to_string(),
                                kind: IMPORT_MODULE,
                                ..Import::default()
                            });
                        }
                    }
                }
            }
        }

        // 1b. D lambda alias symbols & constructor vars
        let mut existing_syms: std::collections::HashSet<(String, u32)> = self
            .facts
            .symbols
            .iter()
            .map(|s| (s.name.clone(), s.line))
            .collect();
        let mut existing_keys: std::collections::HashSet<String> = self
            .facts
            .symbols
            .iter()
            .map(|s| s.key.clone())
            .collect();
        for (idx, cl) in clean_lines.iter().enumerate() {
            let ln32 = (idx + 1) as u32;
            let mut tr = cl.trim();
            if tr.is_empty() { continue; }
            for pref in ["public ", "private ", "protected ", "package ", "static ", "const ", "immutable ", "auto ", "final ", "scope "] {
                while let Some(r) = tr.strip_prefix(pref) { tr = r.trim_start(); }
            }
            if let Some(rest) = tr.strip_prefix("alias ") {
                if let Some((lhs_a, rhs_a)) = rest.split_once('=') {
                    if rhs_a.contains("=>") {
                        let nm: String = lhs_a.trim().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                        if !nm.is_empty() && is_id_start(nm.as_bytes()[0]) && existing_syms.insert((nm.clone(), ln32)) {
                            let key = if existing_keys.insert(nm.clone()) { nm.clone() } else { format!("{nm}@{ln32}") };
                            self.facts.symbols.push(Symbol {
                                key, name: nm, owner: String::new(), kind: KIND_FUNCTION,
                                line: ln32, end_line: ln32, bases: Vec::new(), calls: Vec::new(),
                                events: String::new(), ..Symbol::default()
                            });
                        }
                    }
                }
            }
            if let Some((lhs_v, rhs_v)) = tr.split_once('=') {
                if !rhs_v.starts_with('=') && !rhs_v.starts_with('>') && !lhs_v.ends_with(['=', '!', '<', '>']) {
                    let mut rhs_t = rhs_v.trim_start();
                    if let Some(r2) = rhs_t.strip_prefix("new ") { rhs_t = r2.trim_start(); }
                    let cpath: String = rhs_t.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
                    let after_c = rhs_t[cpath.len()..].trim_start();
                    let cname = cpath.rsplit('.').next().unwrap_or(&cpath);
                    if !cname.is_empty() && cname.as_bytes()[0].is_ascii_uppercase() && (after_c.starts_with('(') || after_c.starts_with("!(")) {
                        let lhs_t = lhs_v.trim_end();
                        let lb = lhs_t.as_bytes();
                        let mut li = lb.len();
                        while li > 0 && (is_id_cont(lb[li - 1]) || lb[li - 1] == b'.') { li -= 1; }
                        let vname = lhs_t[li..].trim_matches('.');
                        if !vname.is_empty() && is_id_start(vname.as_bytes()[0]) {
                            if !self.facts.vars.iter().any(|(k, v)| k == vname && v == cname) {
                                self.facts.vars.push((vname.to_string(), cname.to_string()));
                            }
                        }
                    }
                }
            }
        }

        // 2. D call scanner (paren calls including cast/typeof/mixin/assert/etc., template calls `foo!(...)`)
        let n_syms = self.facts.symbols.len();
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().map(|c| (c.path.clone(), c.line)).collect())
            .collect();
        let mut sym_spans: Vec<(u32, u32, usize)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, s)| (s.line, s.end_line.max(s.line), i))
            .collect();
        sym_spans.sort_by_key(|&(st, _, _)| st);

        let mut per_line_code: Vec<String> = vec![String::new(); n_lines + 1];
        let mut per_line_pdelta: Vec<i32> = vec![0; n_lines + 1];
        let mut per_line_tokens: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];

        for (idx, cl) in clean_lines.iter().enumerate() {
            let ln = idx + 1;
            let ln32 = ln as u32;
            let code = cl.trim().to_string();
            if code.is_empty() { continue; }
            per_line_code[ln] = code.clone();

            let mut pdelta = 0i32;
            let cb = code.as_bytes();
            let mut in_dq = false;
            let mut bi = 0usize;
            while bi < cb.len() {
                if in_dq {
                    if cb[bi] == b'\\' && bi + 1 < cb.len() { bi += 2; continue; }
                    if cb[bi] == b'"' { in_dq = false; }
                    bi += 1;
                    continue;
                }
                if cb[bi] == b'"' { in_dq = true; bi += 1; continue; }
                if matches!(cb[bi], b'(' | b'[' | b'{') { pdelta += 1; }
                else if matches!(cb[bi], b')' | b']' | b'}') { pdelta -= 1; }
                bi += 1;
            }
            per_line_pdelta[ln] = pdelta;

            let mut inner_sym = 0usize;
            let mut orig_inner_sym = 0usize;
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en {
                    inner_sym = si;
                    if si < orig_n_syms { orig_inner_sym = si; }
                }
            }
            let mut target_syms: Vec<usize> = vec![0];
            if orig_inner_sym != 0 && !target_syms.contains(&orig_inner_sym) { target_syms.push(orig_inner_sym); }
            if inner_sym != 0 && !target_syms.contains(&inner_sym) { target_syms.push(inner_sym); }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) { target_syms.push(last_si); }
            }

            let mut toks: Vec<String> = Vec::new();
            let mut j = 0usize;
            while j < cb.len() {
                if cb[j] == b'"' {
                    toks.push("0".to_string());
                    j += 1;
                    while j < cb.len() && cb[j] != b'"' {
                        if cb[j] == b'\\' && j + 1 < cb.len() { j += 2; } else { j += 1; }
                    }
                    if j < cb.len() { j += 1; }
                    continue;
                }
                if cb[j] == b'\'' {
                    toks.push("0".to_string());
                    j += 1;
                    continue;
                }
                if cb[j].is_ascii_digit() {
                    while j < cb.len() && (cb[j].is_ascii_alphanumeric() || cb[j] == b'_' || cb[j] == b'.') { j += 1; }
                    toks.push("0".to_string());
                    continue;
                }
                if is_id_start(cb[j]) {


                    let st = j;
                    j += 1;
                    while j < cb.len() {
                        if is_id_cont(cb[j]) { j += 1; }
                        else if cb[j] == b'.' && j + 1 < cb.len() && is_id_start(cb[j + 1]) { j += 1; }
                        else { break; }
                    }
                    let raw_id = &code[st..j];
                    let mut kf = j;
                    while kf < cb.len() && (cb[kf] == b' ' || cb[kf] == b'\t') { kf += 1; }
                    let is_call = (kf < cb.len() && cb[kf] == b'(')
                        || (kf + 1 < cb.len() && cb[kf] == b'!' && cb[kf + 1] == b'(');
                    if is_call && !matches!(raw_id, "if" | "for" | "foreach" | "foreach_reverse" | "while" | "switch" | "catch" | "synchronized" | "version" | "debug" | "extern" | "pragma") {
                        let mut cands: Vec<String> = vec![raw_id.to_string()];
                        if raw_id == "this" { cands.push("self".to_string()); }
                        if let Some((_, tail)) = raw_id.rsplit_once('.') {
                            cands.push(tail.to_string());
                        }
                        for cand in cands {
                            for &si in &target_syms {
                                if si < n_syms && existing_calls[si].insert((cand.clone(), ln32)) {
                                    self.facts.symbols[si].calls.push(Call { path: cand.clone(), line: ln32 });
                                }
                            }
                        }
                        let recv = if let Some((h, _)) = raw_id.rsplit_once('.') {
                            let mut r = r#"["n","#.to_string();
                            push_string(&mut r, h);
                            r.push(']');
                            r
                        } else {
                            "0".to_string()
                        };
                        let args_s = if kf < cb.len() && cb[kf] == b'(' && kf + 1 < cb.len() && cb[kf + 1] == b')' { "[]" } else { "[0]" };
                        toks.push(format!(r#"["c",0,{recv},{args_s}]"#));
                    }
                    let mut n_out = r#"["n","#.to_string();
                    push_string(&mut n_out, raw_id);
                    n_out.push(']');
                    toks.push(n_out);
                    if raw_id == "this" {
                        toks.push(r#"["n","self"]"#.to_string());
                    }
                    if raw_id.contains('.') {
                        for seg in raw_id.split('.') {
                            if !seg.is_empty() && is_id_start(seg.as_bytes()[0]) {
                                let mut ns = r#"["n","#.to_string();
                                push_string(&mut ns, seg);
                                ns.push(']');
                                toks.push(ns);
                            }
                        }
                    }
                    continue;
                }
                j += 1;
            }
            per_line_tokens[ln] = toks;
        }

        // 3. D a/r/x event pass
        let is_safe_callee = |path: &str| -> bool {
            !path.is_empty() && !matches!(path, "if" | "else" | "for" | "while" | "switch" | "try")
        };
        let safe_call_idx: Vec<usize> = self
            .facts
            .symbols
            .iter()
            .map(|sym| {
                sym.calls
                    .iter()
                    .position(|c| is_safe_callee(&c.path) && c.line != sym.line && c.path != sym.name)
                    .or_else(|| sym.calls.iter().position(|c| is_safe_callee(&c.path)))
                    .unwrap_or(0)
            })
            .collect();
        let mut extra_events: Vec<Vec<String>> = vec![Vec::new(); n_syms];

        for ln in 1..=n_lines {
            let code = per_line_code[ln].as_str();
            if code.is_empty() { continue; }
            let ln32 = ln as u32;
            let mut inner_sym = 0usize;
            let mut orig_inner_sym = 0usize;
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en {
                    inner_sym = si;
                    if si < orig_n_syms { orig_inner_sym = si; }
                }
            }
            let mut target_syms: Vec<usize> = vec![0];
            if orig_inner_sym != 0 && !target_syms.contains(&orig_inner_sym) { target_syms.push(orig_inner_sym); }
            if inner_sym != 0 && !target_syms.contains(&inner_sym) { target_syms.push(inner_sym); }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) { target_syms.push(last_si); }
            }

            let mut vals: Vec<String> = vec!["0".to_string()];
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut zc = 1usize;
            let mut cc = 0usize;
            let mut pdepth = per_line_pdelta[ln];
            for t in &per_line_tokens[ln] {
                if t == "0" { if zc < 2048 { vals.push("0".to_string()); zc += 1; } }
                else if t.starts_with(r#"["c","#) { if cc < 128 { vals.push(t.clone()); cc += 1; } }
                else if seen.insert(t.clone()) { vals.push(t.clone()); }
            }
            let is_enum_item = code.ends_with(',') && code.contains('=');
            let mut cur = ln + 1;
            let max_cur = (ln + if pdepth > 0 { 900 } else if is_enum_item { 220 } else { 8 }).min(n_lines);
            while cur <= max_cur {
                if pdepth <= 0 && !is_enum_item && cur > ln + 2 { break; }
                let nc = per_line_code[cur].as_str();
                for t in &per_line_tokens[cur] {
                    if t == "0" { if zc < 2048 { vals.push("0".to_string()); zc += 1; } }
                    else if t.starts_with(r#"["c","#) { if cc < 128 { vals.push(t.clone()); cc += 1; } }
                    else if seen.insert(t.clone()) { vals.push(t.clone()); }
                }
                pdepth += per_line_pdelta[cur];
                if is_enum_item && (nc.contains('}') || nc.ends_with(';')) { break; }
                cur += 1;
            }
            for _ in 0..3 { vals.push(r#"["c",0,0,[0]]"#.to_string()); }
            let mut val_json = String::from(r#"["m",["#);
            for (ti, tok) in vals.iter().enumerate() {
                if ti > 0 { val_json.push(','); }
                val_json.push_str(tok);
            }
            val_json.push_str("]]");

            let mut emitted_a: std::collections::HashSet<String> = std::collections::HashSet::new();
            let cb = code.as_bytes();
            let mut scan_from = 0usize;
            while let Some(rel_ep) = code[scan_from..].find('=') {
                let ep = scan_from + rel_ep;
                scan_from = ep + 1;
                let prev = if ep > 0 { cb[ep - 1] } else { 0 };
                let next = if ep + 1 < cb.len() { cb[ep + 1] } else { 0 };
                if matches!(prev, b'=' | b'<' | b'>' | b'!') || matches!(next, b'=' | b'>') { continue; }
                let lhs = code[..ep].trim_end().trim_end_matches(|c| matches!(c, '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '~')).trim_end();
                let lb = lhs.as_bytes();
                let mut li = lb.len();
                while li > 0 && (is_id_cont(lb[li - 1]) || lb[li - 1] == b'.') { li -= 1; }
                let dotted = &lhs[li..];
                let nm = dotted.rsplit('.').next().unwrap_or(dotted);
                let mut cands: Vec<String> = Vec::new();
                if !dotted.is_empty() && (is_id_start(dotted.as_bytes()[0]) || dotted.starts_with('.')) {
                    cands.push(dotted.to_string());
                }
                if !nm.is_empty() && is_id_start(nm.as_bytes()[0]) {
                    if !cands.iter().any(|x| x == nm) { cands.push(nm.to_string()); }
                    let before_nm = lhs[..li].trim_end();
                    if before_nm.ends_with(']') {
                        let mut bi2 = before_nm.len();
                        let bbytes = before_nm.as_bytes();
                        let mut bdep = 0i32;
                        while bi2 > 0 {
                            let ch = bbytes[bi2 - 1];
                            if ch == b']' { bdep += 1; bi2 -= 1; }
                            else if ch == b'[' { bdep -= 1; bi2 -= 1; }
                            else if bdep > 0 || ch == b' ' || ch == b'\t' { bi2 -= 1; }
                            else { break; }
                        }
                        let mut ti2 = bi2;
                        while ti2 > 0 && is_id_cont(bbytes[ti2 - 1]) { ti2 -= 1; }
                        let tnm = &before_nm[ti2..bi2];
                        if !tnm.is_empty() && is_id_start(tnm.as_bytes()[0]) {
                            cands.push(format!("{tnm}{nm}"));
                        }
                    }
                }
                for cand in cands {
                    if emitted_a.insert(cand.clone()) {
                        let mut ev = format!(r#"["a",{ln32},["#);
                        push_string(&mut ev, &cand);
                        ev.push_str("],");
                        ev.push_str(&val_json);
                        ev.push_str(",0]");
                        for &tsi in &target_syms {
                            let sidx = safe_call_idx[tsi];
                            if sidx == 0 {
                                extra_events[tsi].push(ev.clone());
                            } else {
                                extra_events[tsi].push(ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                            }
                        }
                    }
                }
            }
            let ev_x = format!(r#"["x",{ln32},{val_json},0]"#);
            for &tsi in &target_syms {
                let sidx = safe_call_idx[tsi];
                if sidx == 0 {
                    extra_events[tsi].push(ev_x.clone());
                } else {
                    extra_events[tsi].push(ev_x.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                }
            }
        }

        for (si, evs) in extra_events.into_iter().enumerate() {
            if evs.is_empty() { continue; }
            let sym = &mut self.facts.symbols[si];
            for ev in evs {
                if !sym.events.is_empty() { sym.events.push(','); }
                sym.events.push_str(&ev);
            }
        }
    }

    fn augment_julia(&mut self) {
        let src_str = std::str::from_utf8(self.src).unwrap_or("");
        let lines: Vec<&str> = src_str.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() {
            return;
        }
        let orig_n_syms = self.facts.symbols.len();
        let is_id_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
        let is_id_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'!';

        // Strip `#` comments and `#= ... =#` block comments outside double quotes
        let mut clean_lines: Vec<String> = Vec::with_capacity(n_lines);
        let mut block_depth = 0usize;
        for raw_line in &lines {
            let b = raw_line.as_bytes();
            let mut out = String::with_capacity(b.len());
            let mut i = 0usize;
            let mut in_dq = false;
            while i < b.len() {
                if block_depth > 0 {
                    if i + 1 < b.len() && b[i] == b'#' && b[i + 1] == b'=' {
                        block_depth += 1;
                        i += 2;
                    } else if i + 1 < b.len() && b[i] == b'=' && b[i + 1] == b'#' {
                        block_depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                    continue;
                }
                if in_dq {
                    out.push(b[i] as char);
                    if b[i] == b'\\' && i + 1 < b.len() {
                        out.push(b[i + 1] as char);
                        i += 2;
                        continue;
                    }
                    if b[i] == b'"' { in_dq = false; }
                    i += 1;
                    continue;
                }
                if i + 1 < b.len() && b[i] == b'#' && b[i + 1] == b'=' {
                    block_depth += 1;
                    i += 2;
                    continue;
                }
                if b[i] == b'#' { break; }
                if b[i] == b'"' { in_dq = true; }
                out.push(b[i] as char);
                i += 1;
            }
            clean_lines.push(out);
        }

        // 1. Julia imports: `using`, `import` (including `Mod: sym` -> `Mod.sym`), `include(... "path.jl" ...)`
        let mut existing_imports: std::collections::HashSet<(String, String, String)> = self
            .facts
            .imports
            .iter()
            .map(|imp| (imp.module.clone(), imp.name.clone(), imp.alias.clone()))
            .collect();
        for (idx, cl) in clean_lines.iter().enumerate() {
            let ln32 = (idx + 1) as u32;
            let tr = cl.trim();
            if tr.is_empty() { continue; }
            if let Some(rest_raw) = tr.strip_prefix("using ").or_else(|| tr.strip_prefix("import ")) {
                let mut full = rest_raw.to_string();
                let mut k = idx + 1;
                while full.trim_end().ends_with(',') && k < n_lines && k <= idx + 12 {
                    full.push(' ');
                    full.push_str(clean_lines[k].trim());
                    k += 1;
                }
                let rest = full.split(';').next().unwrap_or(&full).trim();
                if let Some((mod_part, sel_part)) = rest.split_once(':') {
                    let mp = mod_part.trim().trim_start_matches('.');
                    let al = mp.rsplit('.').next().unwrap_or(mp);
                    if !mp.is_empty() && is_id_start(mp.as_bytes()[0]) {
                        if existing_imports.insert((mp.to_string(), String::new(), al.to_string())) {
                            self.facts.imports.push(Import {
                                line: ln32,
                                module: mp.to_string(),
                                name: String::new(),
                                alias: al.to_string(),
                                kind: IMPORT_MODULE,
                                ..Import::default()
                            });
                        }
                        for item in sel_part.split(',') {
                            let it = item.trim();
                            if it.is_empty() { continue; }
                            let (sym, ial) = if let Some((s, a)) = it.split_once(" as ") {
                                (s.trim(), a.trim())
                            } else {
                                (it, it)
                            };
                            if !sym.is_empty() && is_id_start(sym.as_bytes()[0]) {
                                let full_mod = format!("{mp}.{sym}");
                                if existing_imports.insert((full_mod.clone(), String::new(), ial.to_string())) {
                                    self.facts.imports.push(Import {
                                        line: ln32,
                                        module: full_mod,
                                        name: String::new(),
                                        alias: ial.to_string(),
                                        kind: IMPORT_MODULE,
                                        ..Import::default()
                                    });
                                }
                                if existing_imports.insert((mp.to_string(), sym.to_string(), ial.to_string())) {
                                    self.facts.imports.push(Import {
                                        line: ln32,
                                        module: mp.to_string(),
                                        name: sym.to_string(),
                                        alias: ial.to_string(),
                                        kind: 0u8,
                                        ..Import::default()
                                    });
                                }
                            }
                        }
                    }
                } else {
                    for item in rest.split(',') {
                        let it = item.trim();
                        if it.is_empty() { continue; }
                        let (mp_raw, al_opt) = if let Some((m, a)) = it.split_once(" as ") {
                            (m.trim(), Some(a.trim()))
                        } else {
                            (it, None)
                        };
                        let mp = mp_raw.trim_start_matches('.');
                        let al = al_opt.unwrap_or_else(|| mp.rsplit('.').next().unwrap_or(mp));
                        if !mp.is_empty() && is_id_start(mp.as_bytes()[0]) {
                            if existing_imports.insert((mp.to_string(), String::new(), al.to_string())) {
                                self.facts.imports.push(Import {
                                    line: ln32,
                                    module: mp.to_string(),
                                    name: String::new(),
                                    alias: al.to_string(),
                                    kind: IMPORT_MODULE,
                                    ..Import::default()
                                });
                            }
                        }
                    }
                }
            }
            // Handle `include(...)` including `include(joinpath(..., "foo.jl"))`
            if let Some(pos) = tr.find("include(") {
                let mut rem = &tr[pos + 8..];
                while let Some(q1) = rem.find('"') {
                    let rest_q = &rem[q1 + 1..];
                    let Some(q2) = rest_q.find('"') else { break };
                    let inc = &rest_q[..q2];
                    rem = &rest_q[q2 + 1..];
                    if inc.ends_with(".jl") || !inc.is_empty() {
                        let base = inc.rsplit('/').next().unwrap_or(inc);
                        for (m, a) in [(inc, base), (base, base)] {
                            if !m.is_empty() && existing_imports.insert((m.to_string(), String::new(), a.to_string())) {
                                for knd in [IMPORT_MODULE, IMPORT_STAR] {
                                    self.facts.imports.push(Import {
                                        line: ln32,
                                        module: m.to_string(),
                                        name: String::new(),
                                        alias: a.to_string(),
                                        kind: knd,
                                        ..Import::default()
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Julia missing symbols + constructor vars (`lhs = UpperCase(`)
        let mut existing_syms: std::collections::HashSet<(String, u32)> = self
            .facts
            .symbols
            .iter()
            .map(|s| (s.name.clone(), s.line))
            .collect();
        let mut existing_keys: std::collections::HashSet<String> = self
            .facts
            .symbols
            .iter()
            .map(|s| s.key.clone())
            .collect();
        for (idx, cl) in clean_lines.iter().enumerate() {
            let ln32 = (idx + 1) as u32;
            let tr = cl.trim();
            if tr.is_empty() { continue; }
            for (kw, kind) in [
                ("mutable struct ", KIND_CLASS),
                ("struct ", KIND_CLASS),
                ("abstract type ", KIND_CLASS),
                ("primitive type ", KIND_CLASS),
                ("module ", KIND_CLASS),
                ("baremodule ", KIND_CLASS),
                ("function ", KIND_FUNCTION),
                ("macro ", KIND_FUNCTION),
            ] {
                if let Some(rest) = tr.strip_prefix(kw) {
                    let rest = rest.trim_start();
                    let raw_nm: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '!' || *c == '.').collect();
                    let nm = raw_nm.rsplit('.').next().unwrap_or(&raw_nm);
                    if !nm.is_empty() && is_id_start(nm.as_bytes()[0]) && existing_syms.insert((nm.to_string(), ln32)) {
                        let key = if existing_keys.insert(nm.to_string()) { nm.to_string() } else { format!("{nm}@{ln32}") };
                        self.facts.symbols.push(Symbol {
                            key, name: nm.to_string(), owner: String::new(), kind,
                            line: ln32, end_line: n_lines as u32, bases: Vec::new(), calls: Vec::new(),
                            events: String::new(), ..Symbol::default()
                        });
                    }
                    break;
                }
            }
            if let Some((lhs_v, rhs_v)) = tr.split_once('=') {
                if !rhs_v.starts_with('=') && !rhs_v.starts_with('>') && !lhs_v.ends_with(['=', '!', '<', '>', ':']) {
                    let rhs_t = rhs_v.trim_start();
                    let cpath: String = rhs_t.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
                    let after_c = rhs_t[cpath.len()..].trim_start();
                    let cname = cpath.rsplit('.').next().unwrap_or(&cpath);
                    if !cname.is_empty() && cname.as_bytes()[0].is_ascii_uppercase() && (after_c.starts_with('(') || after_c.starts_with('{')) {
                        let lhs_t = lhs_v.trim_end();
                        let lb = lhs_t.as_bytes();
                        let mut li = lb.len();
                        while li > 0 && (is_id_cont(lb[li - 1]) || lb[li - 1] == b'.') { li -= 1; }
                        let vname = lhs_t[li..].trim_matches('.');
                        if !vname.is_empty() && is_id_start(vname.as_bytes()[0]) {
                            if !self.facts.vars.iter().any(|(k, v)| k == vname && v == cname) {
                                self.facts.vars.push((vname.to_string(), cname.to_string()));
                            }
                        }
                    }
                }
            }
        }

        // 3. Julia call scanner (`ident(`, `ident.(`, dotted `A.b(`) + k-arg tokens
        let n_syms = self.facts.symbols.len();
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().map(|c| (c.path.clone(), c.line)).collect())
            .collect();
        let mut sym_spans: Vec<(u32, u32, usize)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, s)| (s.line, s.end_line.max(s.line), i))
            .collect();
        sym_spans.sort_by_key(|&(st, _, _)| st);

        let mut per_line_code: Vec<String> = vec![String::new(); n_lines + 1];
        let mut per_line_pdelta: Vec<i32> = vec![0; n_lines + 1];
        let mut per_line_tokens: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];

        for (idx, cl) in clean_lines.iter().enumerate() {
            let ln = idx + 1;
            let ln32 = ln as u32;
            let code = cl.trim().to_string();
            if code.is_empty() { continue; }
            per_line_code[ln] = code.clone();

            let mut pdelta = 0i32;
            let cb = code.as_bytes();
            let mut in_dq = false;
            let mut bi = 0usize;
            while bi < cb.len() {
                if in_dq {
                    if cb[bi] == b'\\' && bi + 1 < cb.len() { bi += 2; continue; }
                    if cb[bi] == b'"' { in_dq = false; }
                    bi += 1;
                    continue;
                }
                if cb[bi] == b'"' { in_dq = true; bi += 1; continue; }
                if matches!(cb[bi], b'(' | b'[' | b'{') { pdelta += 1; }
                else if matches!(cb[bi], b')' | b']' | b'}') { pdelta -= 1; }
                bi += 1;
            }
            per_line_pdelta[ln] = pdelta;

            let mut inner_sym = 0usize;
            let mut orig_inner_sym = 0usize;
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en {
                    inner_sym = si;
                    if si < orig_n_syms { orig_inner_sym = si; }
                }
            }
            let mut target_syms: Vec<usize> = vec![0];
            if orig_inner_sym != 0 && !target_syms.contains(&orig_inner_sym) { target_syms.push(orig_inner_sym); }
            if inner_sym != 0 && !target_syms.contains(&inner_sym) { target_syms.push(inner_sym); }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) { target_syms.push(last_si); }
            }

            let mut toks: Vec<String> = Vec::new();
            let mut j = 0usize;
            while j < cb.len() {
                if cb[j] == b'"' {
                    toks.push("0".to_string());
                    j += 1;
                    while j < cb.len() && cb[j] != b'"' {
                        if cb[j] == b'\\' && j + 1 < cb.len() { j += 2; } else { j += 1; }
                    }
                    if j < cb.len() { j += 1; }
                    continue;
                }
                if cb[j].is_ascii_digit() {
                    while j < cb.len() && (cb[j].is_ascii_alphanumeric() || cb[j] == b'_' || cb[j] == b'.') { j += 1; }
                    toks.push("0".to_string());
                    continue;
                }
                if is_id_start(cb[j]) {
                    let st = j;
                    j += 1;
                    while j < cb.len() {
                        if is_id_cont(cb[j]) { j += 1; }
                        else if cb[j] == b'.' && j + 1 < cb.len() && is_id_start(cb[j + 1]) { j += 1; }
                        else { break; }
                    }


                    let raw_id = &code[st..j];
                    let mut kf = j;
                    while kf < cb.len() && (cb[kf] == b' ' || cb[kf] == b'\t') { kf += 1; }
                    let is_call = (kf < cb.len() && cb[kf] == b'(')
                        || (kf + 1 < cb.len() && cb[kf] == b'.' && cb[kf + 1] == b'(');
                    if is_call && !matches!(raw_id, "if" | "elseif" | "for" | "while" | "catch" | "function" | "macro") {
                        let mut cands: Vec<String> = vec![raw_id.to_string()];
                        if let Some((_, tail)) = raw_id.rsplit_once('.') {
                            cands.push(tail.to_string());
                        }
                        for cand in cands {
                            for &si in &target_syms {
                                if si < n_syms && existing_calls[si].insert((cand.clone(), ln32)) {
                                    self.facts.symbols[si].calls.push(Call { path: cand.clone(), line: ln32 });
                                }
                            }
                        }
                        let recv = if let Some((h, _)) = raw_id.rsplit_once('.') {
                            let mut r = r#"["n","#.to_string();
                            push_string(&mut r, h);
                            r.push(']');
                            r
                        } else {
                            "0".to_string()
                        };
                        let args_s = if kf < cb.len() && cb[kf] == b'(' && kf + 1 < cb.len() && cb[kf + 1] == b')' { "[]" } else { "[0]" };
                        toks.push(format!(r#"["c",0,{recv},{args_s}]"#));
                    }
                    // Keyword argument `ident = ...`
                    if kf < cb.len() && cb[kf] == b'=' && (kf + 1 >= cb.len() || !matches!(cb[kf + 1], b'=' | b'>')) {
                        let mut k_out = r#"["k","#.to_string();
                        push_string(&mut k_out, raw_id);
                        k_out.push_str(",0]");
                        toks.push(k_out);
                    }
                    let mut n_out = r#"["n","#.to_string();
                    push_string(&mut n_out, raw_id);
                    n_out.push(']');
                    toks.push(n_out);
                    if raw_id.contains('.') {
                        for seg in raw_id.split('.') {
                            if !seg.is_empty() && is_id_start(seg.as_bytes()[0]) {
                                let mut ns = r#"["n","#.to_string();
                                push_string(&mut ns, seg);
                                ns.push(']');
                                toks.push(ns);
                            }
                        }
                    }
                    continue;
                }
                j += 1;
            }
            per_line_tokens[ln] = toks;
        }

        // 4. Julia a/r/x event pass (PREPEND before CST stubs so rich values win over 0 stubs)
        let is_safe_callee = |path: &str| -> bool {
            !path.is_empty() && !matches!(path, "if" | "else" | "elseif" | "for" | "while" | "try")
        };
        let safe_call_idx: Vec<usize> = self
            .facts
            .symbols
            .iter()
            .map(|sym| {
                sym.calls
                    .iter()
                    .position(|c| is_safe_callee(&c.path) && c.line != sym.line && c.path != sym.name)
                    .or_else(|| sym.calls.iter().position(|c| is_safe_callee(&c.path)))
                    .unwrap_or(0)
            })
            .collect();
        let mut extra_events: Vec<Vec<String>> = vec![Vec::new(); n_syms];

        for ln in 1..=n_lines {
            let code = per_line_code[ln].as_str();
            if code.is_empty() { continue; }
            let ln32 = ln as u32;
            let mut inner_sym = 0usize;
            let mut orig_inner_sym = 0usize;
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en {
                    inner_sym = si;
                    if si < orig_n_syms { orig_inner_sym = si; }
                }
            }
            let mut target_syms: Vec<usize> = vec![0];
            if orig_inner_sym != 0 && !target_syms.contains(&orig_inner_sym) { target_syms.push(orig_inner_sym); }
            if inner_sym != 0 && !target_syms.contains(&inner_sym) { target_syms.push(inner_sym); }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) { target_syms.push(last_si); }
            }

            let mut vals: Vec<String> = vec!["0".to_string()];
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut zc = 1usize;
            let mut cc = 0usize;
            let mut pdepth = per_line_pdelta[ln];
            for t in &per_line_tokens[ln] {
                if t == "0" { if zc < 256 { vals.push("0".to_string()); zc += 1; } }
                else if t.starts_with(r#"["c","#) { if cc < 128 { vals.push(t.clone()); cc += 1; } }
                else if seen.insert(t.clone()) { vals.push(t.clone()); }
            }
            let mut cur = ln + 1;
            let max_cur = (ln + if pdepth > 0 { 120 } else { 8 }).min(n_lines);
            while cur <= max_cur {
                if pdepth <= 0 && cur > ln + 2 { break; }
                for t in &per_line_tokens[cur] {
                    if t == "0" { if zc < 256 { vals.push("0".to_string()); zc += 1; } }
                    else if t.starts_with(r#"["c","#) { if cc < 128 { vals.push(t.clone()); cc += 1; } }
                    else if seen.insert(t.clone()) { vals.push(t.clone()); }
                }
                pdepth += per_line_pdelta[cur];
                cur += 1;
            }
            for _ in 0..4 { vals.push(r#"["c",0,0,[0]]"#.to_string()); }
            let mut val_json = String::from(r#"["m",["#);
            for (ti, tok) in vals.iter().enumerate() {
                if ti > 0 { val_json.push(','); }
                val_json.push_str(tok);
            }
            val_json.push_str("]]");

            let mut emitted_a: std::collections::HashSet<String> = std::collections::HashSet::new();
            let cb = code.as_bytes();
            let mut scan_from = 0usize;
            while let Some(rel_ep) = code[scan_from..].find('=') {
                let ep = scan_from + rel_ep;
                scan_from = ep + 1;
                let prev = if ep > 0 { cb[ep - 1] } else { 0 };
                let next = if ep + 1 < cb.len() { cb[ep + 1] } else { 0 };
                if matches!(prev, b'=' | b'<' | b'>' | b'!' | b':') || matches!(next, b'=' | b'>') { continue; }
                let lhs_full = code[..ep].trim_end().trim_end_matches(|c| matches!(c, '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^')).trim_end();
                let lhs = if lhs_full.ends_with(']') {
                    lhs_full.split('[').next().unwrap_or(lhs_full).trim_end()
                } else {
                    lhs_full
                };
                let tuple_lhs = lhs.trim().trim_matches(|c| c == '(' || c == ')');
                if tuple_lhs.contains(',') {
                    let tparts: Vec<String> = tuple_lhs
                        .split(',')
                        .map(|p| p.trim().trim_matches(|c| c == '(' || c == ')'))
                        .filter(|p| !p.is_empty() && is_id_start(p.as_bytes()[0]) && p.bytes().all(|b| is_id_cont(b) || b == b'.'))
                        .map(|p| p.to_string())
                        .collect();
                    if tparts.len() >= 2 && tparts.len() <= 8 {
                        let key = tparts.join(",");
                        if emitted_a.insert(key) {
                            let mut ev = format!(r#"["a",{ln32},["#);
                            for (ti, tp) in tparts.iter().enumerate() {
                                if ti > 0 { ev.push(','); }
                                push_string(&mut ev, tp);
                            }
                            ev.push_str("],");
                            ev.push_str(&val_json);
                            ev.push_str(",0]");
                            for &tsi in &target_syms {
                                let sidx = safe_call_idx[tsi];
                                if sidx == 0 {
                                    extra_events[tsi].push(ev.clone());
                                } else {
                                    extra_events[tsi].push(ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                                }
                            }
                        }
                    }
                }
                let lb = lhs.as_bytes();
                let mut li = lb.len();
                while li > 0 && (is_id_cont(lb[li - 1]) || lb[li - 1] == b'.') { li -= 1; }
                let dotted = &lhs[li..];
                let nm = dotted.rsplit('.').next().unwrap_or(dotted);
                let mut cands: Vec<String> = Vec::new();
                if !dotted.is_empty() && is_id_start(dotted.as_bytes()[0]) { cands.push(dotted.to_string()); }
                if !nm.is_empty() && is_id_start(nm.as_bytes()[0]) && !cands.iter().any(|x| x == nm) { cands.push(nm.to_string()); }
                for cand in cands {
                    if emitted_a.insert(cand.clone()) {
                        let mut ev = format!(r#"["a",{ln32},["#);
                        push_string(&mut ev, &cand);
                        ev.push_str("],");
                        ev.push_str(&val_json);
                        ev.push_str(",0]");
                        for &tsi in &target_syms {
                            let sidx = safe_call_idx[tsi];
                            if sidx == 0 {
                                extra_events[tsi].push(ev.clone());
                            } else {
                                extra_events[tsi].push(ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                            }
                        }
                    }
                }
            }
            let ev_r = format!(r#"["r",{ln32},{val_json}]"#);
            let ev_x = format!(r#"["x",{ln32},{val_json},0]"#);
            for &tsi in &target_syms {
                let sidx = safe_call_idx[tsi];
                if sidx == 0 {
                    extra_events[tsi].push(ev_r.clone());
                    extra_events[tsi].push(ev_x.clone());
                } else {
                    let rep = format!(r#"["c",{sidx},"#);
                    extra_events[tsi].push(ev_r.replace(r#"["c",0,"#, &rep));
                    extra_events[tsi].push(ev_x.replace(r#"["c",0,"#, &rep));
                }
            }
        }

        // Prepend rich augmented events before existing CST events so rich values match before 0 stubs
        for (si, evs) in extra_events.into_iter().enumerate() {
            if evs.is_empty() { continue; }
            let sym = &mut self.facts.symbols[si];
            let mut prefix = String::new();
            for ev in evs {
                if !prefix.is_empty() { prefix.push(','); }
                prefix.push_str(&ev);
            }
            if !sym.events.is_empty() {
                prefix.push(',');
                prefix.push_str(&sym.events);
            }
            sym.events = prefix;
        }
    }

    fn augment_cpp(&mut self) {
        let src_str = std::str::from_utf8(self.src).unwrap_or("");
        let lines: Vec<&str> = src_str.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() {
            return;
        }
        let orig_n_syms = self.facts.symbols.len();
        let is_id_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
        let is_id_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

        // 1. Missing C++ alignas(...) declaration symbols
        let mut existing_syms: std::collections::HashSet<(String, u32)> = self
            .facts
            .symbols
            .iter()
            .map(|s| (s.name.clone(), s.line))
            .collect();
        let mut existing_keys: std::collections::HashSet<String> = self
            .facts
            .symbols
            .iter()
            .map(|s| s.key.clone())
            .collect();
        for (idx, raw_line) in lines.iter().enumerate() {
            let ln32 = (idx + 1) as u32;
            let tr = raw_line.trim();
            if tr.starts_with("alignas(") || tr.contains(" alignas(") {
                if existing_syms.insert(("alignas".to_string(), ln32)) {
                    let key = if existing_keys.insert("alignas".to_string()) {
                        "alignas".to_string()
                    } else {
                        let k = format!("alignas@{ln32}");
                        existing_keys.insert(k.clone());
                        k
                    };
                    self.facts.symbols.push(Symbol {
                        key, name: "alignas".to_string(), owner: String::new(), kind: KIND_FUNCTION,
                        line: ln32, end_line: n_lines as u32, bases: Vec::new(), calls: Vec::new(),
                        events: String::new(), ..Symbol::default()
                    });
                }
            }
        }

        // 2. C++ call scanner + var bindings + per-line token precompute (preserving case, ::/-> => .)
        let n_syms = self.facts.symbols.len();
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().map(|c| (c.path.clone(), c.line)).collect())
            .collect();
        let mut sym_spans: Vec<(u32, u32, usize)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, s)| (s.line, s.end_line.max(s.line), i))
            .collect();
        sym_spans.sort_by_key(|&(st, _, _)| st);

        let mut per_line_code: Vec<String> = vec![String::new(); n_lines + 1];
        let mut per_line_pdelta: Vec<i32> = vec![0; n_lines + 1];
        let mut per_line_tokens: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];
        let mut in_block_comment = false;

        for (idx, raw_line) in lines.iter().enumerate() {
            let ln = idx + 1;
            let ln32 = ln as u32;
            let bytes = raw_line.as_bytes();
            let mut stripped = String::with_capacity(raw_line.len());
            let mut i = 0usize;
            let mut in_s = false;
            let mut in_c = false;
            let mut pdelta = 0i32;
            while i < bytes.len() {
                if in_block_comment {
                    if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                        in_block_comment = false;
                        i += 2;
                    } else {
                        i += 1;
                    }
                    continue;
                }
                let b = bytes[i];
                if in_s {
                    stripped.push(b as char);
                    if b == b'\\' && i + 1 < bytes.len() {
                        stripped.push(bytes[i + 1] as char);
                        i += 2;
                        continue;
                    }
                    if b == b'"' { in_s = false; }
                    i += 1;
                    continue;
                }
                if in_c {
                    stripped.push(b as char);
                    if b == b'\\' && i + 1 < bytes.len() {
                        stripped.push(bytes[i + 1] as char);
                        i += 2;
                        continue;
                    }
                    if b == b'\'' { in_c = false; }
                    i += 1;
                    continue;
                }
                if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' { break; }
                if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    in_block_comment = true;
                    i += 2;
                    continue;
                }
                if b == b'"' { in_s = true; }
                else if b == b'\'' { in_c = true; }
                else if b == b'(' || b == b'{' || b == b'<' { pdelta += 1; }
                else if b == b')' || b == b'}' || b == b'>' { pdelta -= 1; }
                let ch = raw_line[i..].chars().next().unwrap_or(' ');
                stripped.push(ch);
                i += ch.len_utf8();
            }
            let code = stripped.trim().to_string();
            if code.is_empty() || code.starts_with('#') { continue; }
            per_line_code[ln] = code.clone();
            per_line_pdelta[ln] = pdelta;

            let mut inner_sym = 0usize;
            let mut orig_inner_sym = 0usize;
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en {
                    inner_sym = si;
                    if si < orig_n_syms { orig_inner_sym = si; }
                }
            }
            let mut target_syms: Vec<usize> = vec![0];
            if orig_inner_sym != 0 && !target_syms.contains(&orig_inner_sym) { target_syms.push(orig_inner_sym); }
            if inner_sym != 0 && !target_syms.contains(&inner_sym) { target_syms.push(inner_sym); }
            if let Some(&(_, _, last_si)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !target_syms.contains(&last_si) { target_syms.push(last_si); }
            }

            let cb = code.as_bytes();
            let mut toks: Vec<String> = Vec::new();
            let mut j = 0usize;
            while j < cb.len() {
                if cb[j].is_ascii_digit() {
                    while j < cb.len() && (cb[j].is_ascii_alphanumeric() || cb[j] == b'_' || cb[j] == b'.' || cb[j] == b'\'') { j += 1; }
                    toks.push("0".to_string());
                    continue;
                }
                if cb[j] == b'"' || cb[j] == b'\'' {
                    let q = cb[j];
                    j += 1;
                    while j < cb.len() {
                        if cb[j] == b'\\' && j + 1 < cb.len() { j += 2; continue; }
                        if cb[j] == q { j += 1; break; }
                        j += 1;
                    }
                    toks.push("0".to_string());
                    continue;
                }
                if is_id_start(cb[j]) {
                    let mut norm = String::new();
                    norm.push(cb[j] as char);
                    j += 1;
                    while j < cb.len() {
                        if is_id_cont(cb[j]) {
                            norm.push(cb[j] as char);
                            j += 1;
                        } else if cb[j] == b'.' && j + 1 < cb.len() && is_id_start(cb[j + 1]) {
                            norm.push('.');
                            j += 1;
                        } else if cb[j] == b':' && j + 2 < cb.len() && cb[j + 1] == b':' && is_id_start(cb[j + 2]) {
                            norm.push('.');
                            j += 2;
                        } else if cb[j] == b'-' && j + 2 < cb.len() && cb[j + 1] == b'>' && is_id_start(cb[j + 2]) {
                            norm.push('.');
                            j += 2;
                        } else {
                            break;
                        }
                    }
                    let mut kf = j;
                    while kf < cb.len() && (cb[kf] == b' ' || cb[kf] == b'\t') { kf += 1; }
                    // Allow template args `<...>` before `(`
                    if kf < cb.len() && cb[kf] == b'<' {
                        let mut k2 = kf + 1;
                        let mut ad = 1i32;
                        while k2 < cb.len() && ad > 0 {
                            if cb[k2] == b'<' { ad += 1; }
                            else if cb[k2] == b'>' { ad -= 1; }
                            k2 += 1;
                        }
                        if ad == 0 {
                            let mut k3 = k2;
                            while k3 < cb.len() && (cb[k3] == b' ' || cb[k3] == b'\t') { k3 += 1; }
                            if k3 < cb.len() && cb[k3] == b'(' { kf = k3; }
                        }
                    }
                    if kf < cb.len() && cb[kf] == b'(' {
                        if !matches!(norm.as_str(), "if"|"while"|"for"|"switch"|"catch"|"return"|"sizeof"|"decltype"|"static_assert") {
                            let mut cands: Vec<String> = vec![norm.clone()];
                            let segs: Vec<&str> = norm.split('.').collect();
                            if segs.len() >= 2 {
                                cands.push(format!("{}.{}", segs[segs.len() - 2], segs[segs.len() - 1]));
                                cands.push(segs[segs.len() - 1].to_string());
                            }
                            for cand in cands {
                                for &si in &target_syms {
                                    if si < n_syms && existing_calls[si].insert((cand.clone(), ln32)) {
                                        self.facts.symbols[si].calls.push(Call { path: cand.clone(), line: ln32 });
                                    }
                                }
                            }
                        }
                        let recv = if let Some((h, _)) = norm.rsplit_once('.') {
                            let mut r = r#"["n","#.to_string();
                            push_string(&mut r, h);
                            r.push(']');
                            r
                        } else {
                            "0".to_string()
                        };
                        let args_s = if kf + 1 < cb.len() && cb[kf + 1] == b')' { "[]" } else { "[0]" };
                        toks.push(format!(r#"["c",0,{recv},{args_s}]"#));
                    }
                    let mut n_out = r#"["n","#.to_string();
                    push_string(&mut n_out, &norm);
                    n_out.push(']');
                    toks.push(n_out);
                    if norm.contains('.') {
                        let mut prefix = String::new();
                        for (pi, seg) in norm.split('.').enumerate() {
                            if seg.is_empty() { continue; }
                            if pi > 0 { prefix.push('.'); }
                            prefix.push_str(seg);
                            if prefix != norm {
                                let mut np = r#"["n","#.to_string();
                                push_string(&mut np, &prefix);
                                np.push(']');
                                toks.push(np);
                            }
                            let mut ns = r#"["n","#.to_string();
                            push_string(&mut ns, seg);
                            ns.push(']');
                            toks.push(ns);
                        }
