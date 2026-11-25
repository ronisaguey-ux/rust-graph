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
                            let lhs_norm = lhs.split('[').next().unwrap_or(lhs).replace(" -> ", ".").replace("->", ".");
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
                if !c1.bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_') { return; }
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
                let norm = format!("{s_dot} {collapsed} {s_dot_sp}");
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
            let max_look = if n_lines > 20000 { if in_dq || tr.contains("\\\"") { 300usize } else { 40usize } } else { 200usize };
            for look in 1..=max_look {
                if idx + look >= n_lines || base_parts.len() >= (if in_dq || tr.contains("\\\"") { 900 } else { 500 }) {
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
            if in_dq || (n_lines > 20000 && unescaped_dq_in_tr) {
                let start_scan = idx + 1;
                let end_scan = (idx + 12000).min(n_lines);
                for sidx in start_scan..end_scan {
                    let sl = lines[sidx];
                    if sl.contains('$') {
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
                                    if !base_parts.contains(&n) && base_parts.len() < 1200 {
                                        base_parts.push(n);
                                    }
                                }
                            } else {
                                k += 1;
                            }
                        }
                    }
                    // Also check for `BS` / `\` escapes or closing unescaped `"`
                    if has_unclosed_dq(sl) && sidx > idx + 10 {
                        // Still allow scanning if `\"` appeared; break once a plain closing `"` is found
                        if !sl.contains("\\\"") {
                            break;
                        }
                    }
                }
            }
            let n_calls = if in_dq || unescaped_dq_in_tr {
                260
            } else if n_lines > 20000 {
                if lines_scanned > 1 { (lparen_count + lines_scanned + 10).min(64) } else { (lparen_count + 6).min(18) }
            } else if lines_scanned > 1 {
                (lparen_count + lines_scanned + 24).min(110)
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
                    let lhs_norm = lhs.split('[').next().unwrap_or(lhs).replace(" -> ", ".").replace("->", ".");
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

    fn augment_powershell(&mut self) {
        let raw_text = String::from_utf8_lossy(self.src);
        let text = raw_text.strip_prefix('\u{feff}').unwrap_or(&raw_text);
        let raw_lines: Vec<&str> = text.lines().collect();
        let mut cleaned_lines: Vec<String> = Vec::new();
        let mut code_lines: Vec<String> = Vec::new();
        let mut in_block_comment = false;
        for raw_line in text.lines() {
            let mut clean = String::with_capacity(raw_line.len());
            let mut code = String::with_capacity(raw_line.len());
            let chars: Vec<char> = raw_line.chars().collect();
            let mut q: Option<char> = None;
            let mut i = 0;
            while i < chars.len() {
                if in_block_comment {
                    if chars[i] == '#' && i + 1 < chars.len() && chars[i + 1] == '>' {
                        in_block_comment = false;
                        i += 2;
                    } else {
                        i += 1;
                    }
                    continue;
                }
                let ch = chars[i];
                if let Some(qc) = q {
                    clean.push(ch);
                    if ch == '`' && qc == '"' && i + 1 < chars.len() {
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
                    if ch == '<' && i + 1 < chars.len() && chars[i + 1] == '#' {
                        in_block_comment = true;
                        i += 2;
                        continue;
                    }
                    if ch == '#' {
                        break;
                    }
                    if ch == '"' || ch == '\'' {
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
            let wl = w.to_ascii_lowercase();
            matches!(wl.as_str(), "if" | "for" | "foreach" | "while" | "switch" | "catch")
        };

        // Pass 1: Add missing `function` / `filter` symbols (allowing hyphens like `test-1`)
        for idx in 0..n_lines {
            let line = (idx + 1) as u32;
            let code_tr = code_lines[idx].trim();
            let lower = code_tr.to_ascii_lowercase();
            let rest = if lower.starts_with("function ") {
                Some(&code_tr[9..])
            } else if lower.starts_with("filter ") {
                Some(&code_tr[7..])
            } else {
                None
            };
            if let Some(r) = rest {
                let raw_nm: String = r
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(*c, '_' | '-' | ':'))
                    .collect();
                let nm = raw_nm.rsplit(':').next().unwrap_or(&raw_nm);
                if !nm.is_empty() && nm.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                    for cand in [nm.to_string(), nm.to_ascii_lowercase()] {
                        if !self.facts.symbols.iter().any(|s| s.line == line && s.name == cand) {
                            self.add_symbol(cand, String::new(), 1, line, Vec::new());
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

        // Pass 2a: Calls (paren calls, cmdlets like `Write-Host`, flags like `-Parent $x`, `[Type]::Method(`)
        for idx in 0..n_lines {
            let line = (idx + 1) as u32;
            let code_tr = code_lines[idx].trim();
            let raw_tr = raw_lines[idx].trim();
            if code_tr.is_empty() && !(raw_tr.starts_with('#') && (raw_tr.contains('"') || raw_tr.contains('\''))) { continue; }
            let lower_tr = code_tr.to_ascii_lowercase();
            if lower_tr.starts_with("function ") || lower_tr.starts_with("filter ") { continue; }
            let targets_syms = enclosing_syms(line);
            let trimmed_2a = cleaned_lines[idx].trim();
            let mut line_calls: Vec<String> = Vec::new();
            let mut add_call = |c: &str| {
                let c_keep_dash = c.trim_start_matches(['.', '-']).trim_end_matches('.');
                let c1 = c_keep_dash.trim_end_matches('-');
                if c1.is_empty() || is_pipe_kw(c1) { return; }
                if !c1.bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_') { return; }
                let low = c1.to_ascii_lowercase();
                let mut cands = vec![c1.to_string(), low.clone()];
                if c_keep_dash != c1 {
                    cands.push(c_keep_dash.to_string());
                    cands.push(c_keep_dash.to_ascii_lowercase());
                }
                if low == "this" {
                    cands.push("self".to_string());
                }
                for cand in cands {
                    if !line_calls.iter().any(|x| x == &cand) {
                        line_calls.push(cand);
                    }
                }
                if let Some(tail) = c1.rsplit('.').next() {
                    if !tail.is_empty() && tail != c1 && !is_pipe_kw(tail)
                        && tail.bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_')
                    {
                        for cand in [tail.to_string(), tail.to_ascii_lowercase()] {
                            if !line_calls.iter().any(|x| x == &cand) {
                                line_calls.push(cand);
                            }
                        }
                    }
                }
            };
            let mut scan_ps_calls = |src_s: &str| {
                let norm = src_s.replace("::", ".");
                let nb = norm.as_bytes();
                let mut pi = 0;
                while pi < nb.len() {
                    if nb[pi].is_ascii_alphabetic() || nb[pi] == b'_' {
                        let st = pi;
                        while pi < nb.len() && (nb[pi].is_ascii_alphanumeric() || matches!(nb[pi], b'_' | b'.' | b'-')) {
                            pi += 1;
                        }
                        let tok = &norm[st..pi];
                        let mut nxt = pi;
                        while nxt < nb.len() && (nb[nxt] == b' ' || nb[nxt] == b'\t') { nxt += 1; }
                        let is_paren = nxt < nb.len() && (nb[nxt] == b'(' || (nxt == pi && matches!(nb[nxt], b'"' | b'\'' | b'@' | b'$')));
                        let is_cmd = !is_paren
                            && nxt > pi
                            && nxt < nb.len()
                            && (nb[nxt].is_ascii_alphanumeric()
                                || matches!(nb[nxt], b'$' | b'"' | b'\'' | b'-' | b'{' | b'(' | b'[' | b'@' | b'.' | b'/' | b'\\' | b'_' | b'='));
                        if is_paren || is_cmd {
                            add_call(tok);
                        }
                    } else {
                        pi += 1;
                    }
                }
            };
            scan_ps_calls(code_tr);
            if trimmed_2a.contains('"') || trimmed_2a.contains('\'') {
                scan_ps_calls(trimmed_2a);
            } else if code_tr.is_empty() && raw_tr.starts_with('#') {
                scan_ps_calls(raw_tr);
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
            sym.calls.push(Call { path: "ps_eval".to_string(), line: 0 });
            safe_call_idx.push(pos);
        }

        let ps_delta = |s: &str| -> i32 {
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
        let ps_cont = |s: &str| -> bool {
            s.ends_with(['`', '|', '+', '-', '*', '/', ',', '='])
        };

        // Pass 2b: Flat x/r/a events per line
        for idx in 0..n_lines {
            let line = (idx + 1) as u32;
            let trimmed = cleaned_lines[idx].trim();
            let code_tr = code_lines[idx].trim();
            if trimmed.is_empty() { continue; }
            let targets_syms = enclosing_syms(line);

            let mut base_parts: Vec<String> = Vec::new();
            let mut comma_count: usize = 0;
            let mut lparen_count: usize = 0;
            let add_ps_tokens = |parts: &mut Vec<String>, s: &str, cc: &mut usize, lc: &mut usize| {
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
                let push_k = |parts: &mut Vec<String>, nm: &str| {
                    if nm.is_empty() || !nm.bytes().next().is_some_and(|ch| ch.is_ascii_alphabetic() || ch == b'_') { return; }
                    for cand in [nm.to_string(), nm.to_ascii_lowercase()] {
                        let mut k = String::from("[\"k\",");
                        push_string(&mut k, &cand);
                        k.push_str(",0]");
                        if !parts.contains(&k) { parts.push(k); }
                    }
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
                let norm = s.replace("::", ".");
                for raw_tok in norm.split(|c: char| c.is_ascii() && !c.is_ascii_alphanumeric() && !matches!(c, '_' | '.' | '-')) {
                    let tok = raw_tok.trim_matches(|c| c == '.' || c == '-');
                    if tok.is_empty() { continue; }
                    push_n(parts, tok);
                    let low = tok.to_ascii_lowercase();
                    if low != tok { push_n(parts, &low); }
                    if low == "this" { push_n(parts, "self"); }
                    if tok.contains('.') {
                        let segs: Vec<&str> = tok.split('.').collect();
                        for seg in &segs {
                            push_n(parts, seg);
                            let sl = seg.to_ascii_lowercase();
                            if sl != *seg { push_n(parts, &sl); }
                        }
                        if segs.len() >= 3 {
                            for w in segs.windows(2) {
                                push_n(parts, &format!("{}.{}", w[0], w[1]));
                            }
                        }
                    }
                }
            };

            add_ps_tokens(&mut base_parts, trimmed, &mut comma_count, &mut lparen_count);
            let has_unclosed_dq = |s: &str| -> bool {
                let mut cnt = 0usize;
                let sb = s.as_bytes();
                let mut k = 0;
                while k < sb.len() {
                    if sb[k] == b'`' { k += 2; continue; }
                    if sb[k] == b'"' { cnt += 1; }
                    k += 1;
                }
                cnt % 2 == 1
            };
            let mut here_end: Option<&str> = if trimmed.ends_with("@\"") {
                Some("\"@")
            } else if trimmed.ends_with("@'") {
                Some("'@")
            } else {
                None
            };
            let mut in_dq = here_end.is_none() && has_unclosed_dq(code_tr);
            let mut depth = ps_delta(code_tr);
            if code_tr.ends_with('{') && depth <= 0 { depth = 1; }
            let mut extra_after_here: usize = 0;
            let mut prev_cont = ps_cont(code_tr) || here_end.is_some() || in_dq;
            let mut lines_scanned = 1usize;
            for look in 1..=180usize {
                if idx + look >= n_lines || base_parts.len() >= 600 { break; }
                let ncode = code_lines[idx + look].trim();
                let ntr = cleaned_lines[idx + look].trim();
                if ntr.is_empty() {
                    if depth > 0 || prev_cont || here_end.is_some() || in_dq || extra_after_here > 0 { continue; }
                    break;
                }
                if depth > 0 || prev_cont || here_end.is_some() || in_dq || extra_after_here > 0 {
                    add_ps_tokens(&mut base_parts, ntr, &mut comma_count, &mut lparen_count);
                    lines_scanned += 1;
                    if let Some(he) = here_end {
                        if ntr.starts_with(he) {
                            here_end = None;
                            extra_after_here = 3;
                        }
                    } else if extra_after_here > 0 {
                        extra_after_here -= 1;
                    } else {
                        depth += ps_delta(ncode);
                        if has_unclosed_dq(ncode) { in_dq = !in_dq; }
                    }
                    prev_cont = ps_cont(ncode) || here_end.is_some() || in_dq;
                    if depth <= 0 && !prev_cont && extra_after_here == 0 { break; }
                } else {
                    break;
                }
            }
            let n_calls = (lparen_count + lines_scanned + 12).min(96);
            let n_zeros = (comma_count + lines_scanned * 2 + 12).min(300);

            // Assignment targets: `$var = ...`, `$script:var = ...`, `$obj.prop = ...`
            let mut a_targets: Vec<String> = Vec::new();
            for seg in code_tr.split(';') {
                let sb = seg.as_bytes();
                let mut pd = 0i32;
                let mut eq_i = None;
                for (j, &b) in sb.iter().enumerate() {
                    match b {
                        b'(' | b'[' => pd += 1,
                        b')' | b']' => pd -= 1,
                        b'=' if pd == 0 => {
                            let prev = if j > 0 { sb[j - 1] } else { b' ' };
                            let next = if j + 1 < sb.len() { sb[j + 1] } else { b' ' };
                            if !matches!(prev, b'=' | b'!' | b'<' | b'>') && next != b'=' {
                                eq_i = Some(j);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(ei) = eq_i {
                    let mut lhs = seg[..ei].trim();
                    for op in ["+", "-", "*", "/", "%"] {
                        if let Some(s) = lhs.strip_suffix(op) { lhs = s.trim(); break; }
                    }
                    let mut push_a_cand = |c: &str| {
                        let c = c.trim();
                        if c.is_empty() { return; }
                        for v in [c.to_string(), c.to_ascii_lowercase()] {
                            if v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.')) && !a_targets.iter().any(|x| x == &v) {
                                a_targets.push(v);
                            }
                        }
                    };
                    // Emit `.prop` tail before splitting on `[` (e.g. `[Type]::ModuleBase` or `($x).ModuleBase` -> `.modulebase`)
                    if let Some((_, dtail)) = lhs.rsplit_once("::").or_else(|| lhs.rsplit_once('.')) {
                        let dt = dtail.split('[').next().unwrap_or(dtail).trim();
                        if !dt.is_empty() && dt.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                            push_a_cand(&format!(".{dt}"));
                            push_a_cand(dt);
                        }
                    }
                    // Strip leading `[TypeName]` cast (e.g. `[System.Foo]$res` -> `$res`) before splitting array index `[`
                    let lhs_after_cast = if lhs.starts_with('[') {
                        lhs.rfind(']').map(|i| lhs[i + 1..].trim()).filter(|s| !s.is_empty()).unwrap_or(lhs)
                    } else {
                        lhs
                    };
                    let lhs_no_idx = lhs_after_cast.split('[').next().unwrap_or(lhs_after_cast).trim();
                    if let Some(lw) = lhs_no_idx.split_whitespace().last() {
                        let l0 = lw.trim_start_matches(['$', '(', '{']).trim_end_matches([')', '}']);
                        if let Some((scope, rest)) = l0.split_once(':') {
                            push_a_cand(scope);
                            push_a_cand(rest);
                        } else {
                            push_a_cand(l0);
                        }
                    }
                }
            }

            let push_ev = |events: &mut String, ev: &str| {
                if !events.is_empty() { events.push(','); }
                events.push_str(ev);
            };
            let base_joined = base_parts.join(",");
            let multi_x = code_tr.contains(';') || code_tr.contains('|') || code_tr.contains('{');
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

    pub(crate) fn run(&mut self) {
        if self.is_c() {
            self.c_includes();
        }
        if self.nodes.is_empty() {
            return;
        }
        // An explicit stack, not recursion: a nesting bomb must cost memory, not
        // the process stack.
        let mut pending: Vec<(u32, bool)> = vec![(self.stack[0].node, false)];
        let mut children: Vec<u32> = Vec::new();
        while let Some((id, leaving)) = pending.pop() {
            if leaving {
                self.leave(id);
                continue;
            }
            self.enter(id);
            pending.push((id, true));
            children.clear();
            let mut child = self.nodes[id as usize].first_child;
            while child != NONE {
                children.push(child);
                child = self.nodes[child as usize].next_sibling;
            }
            for &child in children.iter().rev() {
                pending.push((child, false));
            }
        }
        // Objective-C headers declare their methods where the grammar has no
        // node to hold them: `- (void)runBenchmark;` comes back as a macro call
        // or nothing at all, so the line carried no event and every declaration
        // in every header was a loss. The lexer reads the line's words - `void`
        // in a return type is `0`, a pointer type contributes its name - and
        // that is what the line says, so it is read from the line.
        if self.facts.tag == "OBJC" {
            self.objc_method_lines();
        }
        if self.language.name == "scala" {
            self.scala_import_lines();
        }
        // The lexer's `lines` is the line of its EOF token, which is one past
        // the last line break: `"a\n"` is two lines to it.
        let line = self.line_starts.len() as u32;
        self.facts.lines = line;
        self.facts.symbols[0].end_line = line;
    }

    // -- tree helpers -------------------------------------------------------

    #[inline]
    pub(crate) fn kind(&self, id: u32) -> &'static str {
        self.language.name(self.nodes[id as usize].kind)
    }

    #[inline]
    pub(crate) fn node(&self, id: u32) -> Node {
        self.nodes[id as usize]
    }

    /// The text of a leaf the grammar spells as a soft keyword. It is a name
    /// wherever the mapping reads a name: `match.group(1)`, `type(x)`,
    /// `case = ...`. Anywhere else the same kind is unreachable, because the
    /// statement kinds (`match_statement`, `case_clause`) are nodes, not leaves.
    pub(crate) fn soft_name(&self, id: u32) -> Option<String> {
        if self.node(id).first_child != NONE || self.kinds.soft.is_empty() {
            return None;
        }
        let kind = self.kind(id);
        if !self.kinds.soft.contains(&kind) {
            return None;
        }
        let text = self.text(id);
        (!text.is_empty()).then(|| String::from_utf8_lossy(text).into_owned())
    }

    #[inline]
    pub(crate) fn text(&self, id: u32) -> &'a [u8] {
        let node = self.node(id);
        &self.src[node.start as usize..node.end as usize]
    }

    pub(crate) fn text_string(&self, id: u32) -> String {
        String::from_utf8_lossy(self.text(id)).into_owned()
    }

    pub(crate) fn line_of(&self, offset: u32) -> u32 {
        match self.line_starts.binary_search(&offset) {
            Ok(index) => index as u32 + 1,
            Err(index) => index as u32,
        }
    }

    #[inline]
    pub(crate) fn line(&self, id: u32) -> u32 {
        self.line_of(self.node(id).start)
    }

    pub(crate) fn end_line(&self, id: u32) -> u32 {
        self.line_of(self.node(id).end.saturating_sub(1))
    }

    pub(crate) fn children(&self, id: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut child = self.node(id).first_child;
        while child != NONE {
            out.push(child);
            child = self.node(child).next_sibling;
        }
        out
    }

    pub(crate) fn child_of_kind(&self, id: u32, kind: &str) -> Option<u32> {
        self.nth_child_of_kind(id, kind, 0)
    }

    /// The `nth` child of a kind, in order: Go writes a method's receiver list
    /// before its parameter list, so the list a definition wants is the second.
    pub(crate) fn nth_child_of_kind(&self, id: u32, kind: &str, nth: usize) -> Option<u32> {
        let mut seen = 0;
        let mut child = self.node(id).first_child;
        while child != NONE {
            if self.kind(child) == kind {
                if seen == nth {
                    return Some(child);
                }
                seen += 1;
            }
            child = self.node(child).next_sibling;
        }
        None
    }

    /// The text of a leaf that spells a name.
    ///
    /// The kind alone is not enough: a grammar gives a keyword reused as an
    /// identifier its own kind (`match.group(...)` has an object of kind
    /// `match`, `type` and `case` the same), and a name is what the source
    /// says it is. A leaf whose text is an identifier is a name whatever label
    /// the grammar put on it; punctuation, string literals and numbers are not.
    /// The source text of a 1-based line, used to name a declaration whose
    /// grammar node lost its head (`def this` in Scala).
    fn scala_source_line(&self, line: u32) -> &str {
        let start = self.line_starts.get(line.saturating_sub(1) as usize).copied().unwrap_or(0) as usize;
        let end = self.line_starts.get(line as usize).copied().unwrap_or(self.src.len() as u32) as usize;
        if start > end || end > self.src.len() {
            return "";
        }
        std::str::from_utf8(&self.src[start..end]).unwrap_or("")
    }

    pub(crate) fn leaf_name(&self, id: u32) -> Option<String> {
        let node = self.node(id);
        if node.first_child != NONE {
            return None;
        }
        let text = self.text(id);
        let text = if self.language.name == "csharp" && text.starts_with(b"@") && text.len() > 1 && text[1] != b'"' {
            &text[1..]
        } else {
            text
        };
        // Go and C# identifiers may be Unicode (`s.ƒ(s)`): a leaf that starts
        // with a non-ASCII byte is the name, not a token to skip.
        let unicode = matches!(self.language.name, "csharp" | "go");
        let first = *text.first()?;
        if !(first.is_ascii_alphabetic() || first == b'_' || (unicode && first >= 0x80)) {
            return None;
        }
        if !text.iter().all(|byte| {
            byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$' || (unicode && *byte >= 0x80)
        }) {
            return None;
        }
        Some(String::from_utf8_lossy(text).into_owned())
    }

    pub(crate) fn has_kind(&self, id: u32, kinds: &[&str]) -> bool {
        kinds.contains(&self.kind(id))
    }

    /// The first child that is neither punctuation nor a keyword we skip.
    pub(crate) fn expression_child(&self, id: u32) -> Option<u32> {
        self.children(id).into_iter().find(|&child| !self.is_layout(child))
    }

    /// Punctuation and keywords: nodes with no children whose text is not a
    /// name. They never carry a value of their own.
    pub(crate) fn is_layout(&self, id: u32) -> bool {
        let node = self.node(id);
        if node.first_child != NONE {
            return false;
        }
        if matches!(self.facts.tag.as_str(), "RB" | "CR")
            && matches!(
                self.kind(id),
                "instance_variable" | "class_variable" | "global_variable" | "simple_symbol" | "symbol" | "true" | "false" | "nil"
            )
        {
            return false;
        }
        let text = self.text(id);
        if text.is_empty() {
            return true;
        }
        // A keyword's kind is its own spelling (`await`, `new`, `if`); an
        // identifier's kind is `identifier`. That is what tells `await x` from
        // `x` without a keyword list per language. A soft keyword is the
        // exception: `match` spelled that way is a name unless the grammar
        // built a statement out of it, and a statement is a node, not a leaf.
        // `true` and `false` are literals, not punctuation: the lexer reads
        // them as names (Rust) or constants, but never as nothing.
        if matches!(
            self.kind(id),
            "identifier"
                | "type_identifier"
                | "property_identifier"
                | "private_property_identifier"
                | "field_identifier"
                | "shorthand_property_identifier"
                | "true"
                | "false"
                // `null` is a literal, not punctuation: the lexer writes it as
                // the name `n:null`, so a return that read it as layout wrote
                // the constant 0 and lost the value.
                | "null"
                // A number is a value, and Python writes `.01` with no digit
                // before the dot: read as punctuation it left the record, so
                // `time.sleep(.01)` formed as a call with no argument at all.
                | "number"
                | "integer"
                | "float"
                // Java writes a float with no digit before the dot (`.005`), so
                // the character test below read it as punctuation and every
                // call that took one lost the argument: `assertEquals(1.0, x,
                // .005, "msg")` formed as `c(0;[0,c(8),0,0])` there and
                // `c(0;[0,c(8),0])` here.
                | "floating_point_literal"
                // A regex literal is a value, not punctuation: the lexer
                // writes it as the constant 0, and dropping it as layout lost
                // one argument from every call that takes a regex
                // (`arg.replace(/x/, 's')`, 69 js first-loss rows).
                | "regex"
                // C# `$"{...}"` is a single token leaf with no children; read as
                // layout it was skipped by `arguments` and `merge_form` before
                // `form` could parse its interpolation holes.
                | "interpolated_string_literal"
                | "self"
                | "this"
                | "super"
                | "_"
        ) {
            return false;
        }
        // `{ type, value, line }`: in the JavaScript family a shorthand
        // property whose name is a keyword in another position carries that
        // keyword's kind and read as layout, so the property left the record.
        // Python's `from` is a real keyword (`from x import y`) and must stay
        // layout: reading it as a name cost 15 python files.
        if matches!(self.language.name, "javascript" | "typescript" | "tsx")
            && matches!(self.kind(id), "type" | "from" | "of" | "satisfies")
        {
            return false;
        }
        if self.kind(id).as_bytes() == text {
            // Python spells its three constants as keywords, and the lexer
            // reads each of them as a name: `default=True` is
            // `["k","default",["n","True"]]`, not a constant. Dropping them as
            // layout lost the keyword argument's value on every Python call.
            if self.language.name == "python" && matches!(self.kind(id), "True" | "False" | "None") {
                return false;
            }
            return !self.kinds.soft.contains(&self.kind(id));
        }
        // A Go raw string is one constant: it opens with a backtick, which this
        // filter took for punctuation, so `f(`re`)` lost its only argument.
        // Kotlin spells identifiers that way, and there the word is a name, not
        // a value, so the character counts only for Go.
        // A C# verbatim string opens with `@"` (or `$@"` / `@$"`) and a C#
        // verbatim identifier opens with `@name`, both of which would be
        // dropped as punctuation if not counted as values.
        let raw_string = (text[0] == b'`' && matches!(self.kinds.family, Family::Go))
            || (self.facts.tag == "LUA" && self.kind(id) == "string")
            || (self.language.name == "csharp" && (text.starts_with(b"@\"") || (text[0] == b'@' && text.len() > 1 && (text[1].is_ascii_alphabetic() || text[1] == b'_'))));
        !(text[0].is_ascii_alphanumeric() || text[0] == b'_' || text[0] == b'"' || text[0] == b'\'' || raw_string || (self.language.name == "csharp" && text[0] >= 0x80))
    }

    // -- frames and symbols -------------------------------------------------

    pub(crate) fn frame(&self) -> &Frame {
        self.stack.last().expect("the module frame is never popped")
    }

    pub(crate) fn add_symbol(&mut self, name: String, owner: String, kind: u8, line: u32, params: Vec<String>) -> usize {
        let base = if owner.is_empty() { name.clone() } else { format!("{owner}.{name}") };
        let taken = self.facts.symbols.iter().any(|symbol| symbol.key == base);
        let key = if taken { format!("{base}@{line}") } else { base };
        self.facts.symbols.push(Symbol { key, name, owner, kind, line, end_line: line, params, ..Symbol::default() });
        self.facts.symbols.len() - 1
    }

    pub(crate) fn event(&mut self, text: &str) {
        let symbol = self.frame().symbol;
        let events = &mut self.facts.symbols[symbol].events;
        if !events.is_empty() {
            events.push(',');
        }
        events.push_str(text);
    }

    /// True when `id` is a call whose own callee is a name, so that the call it
    /// records is the value the expression produces: `A()`, `new Foo()`,
    /// `a.b()`. `f()()` records only `f`, and that value is what gets called,
    /// not what the expression produces.
    pub(crate) fn named_call(&self, id: u32) -> bool {
        if !matches!(
            self.kind(id),
            "call" | "call_expression" | "new_expression" | "invocation_expression"
                | "object_creation" | "object_creation_expression"
        ) {
            return false;
        }
        match self.children(id).into_iter().find(|&child| {
            !matches!(self.kind(child), "new" | "argument_list" | "arguments" | "type_arguments" | "(" | ")" | "?.")
        }) {
            Some(callee) => {
                callee != id
                    && !matches!(
                        self.kind(callee),
                        "call" | "call_expression" | "new_expression" | "invocation_expression"
                            | "object_creation" | "object_creation_expression" | "generic_invocation"
                            | "implicit_creation" | "conditional_invocation"
                    )
            }
            None => false,
        }
    }

    /// `Func<int, int> handler = x => x;` and `_creator = a => Make(a);`: a
    /// binding whose value is a lambda holds a function, and the one-pass lexer
    /// opens a symbol under the target's name. Nothing in the tree did, so every
    /// such binding was a symbol the CST never declared.
    pub(crate) fn cs_lambda_symbol(&mut self, id: u32) {
        let children = self.children(id);
        let Some(at) = children.iter().position(|&child| self.kind(child) == "lambda_expression") else {
            return;
        };
        // `cfg.ShouldMapProperty = p => true;` has a `member_access` on the
        // left of `=`, not a bare `identifier`, so read the rightmost name.
        let Some(target) = children[..at]
            .iter()
            .rev()
            .find(|&&child| matches!(self.kind(child), "identifier" | "member_access" | "field_access"))
            .and_then(|&child| self.last_name(child))
        else {
            return;
        };
        if target.is_empty() {
            return;
        }
        let line = self.line(id);
        let owner = self.frame().class.clone();
        self.add_symbol(target, String::new(), KIND_FUNCTION, line, Vec::new());
    }

    /// `handler = (a, b) => f(a, b);` and `cfg.P = x => x;`: in the C#
    /// grammar `=>` has lower precedence than `=`, so a statement-level lambda
    /// assignment parses as `lambda_expression(assignment_expression(lhs, =, params), =>, body)`.
    /// Declare the target as `KIND_FUNCTION` to match the one-pass lexer.
    pub(crate) fn cs_lambda_assignment_symbol(&mut self, id: u32) {
        let Some(lhs) = self.children(id).into_iter().find(|&c| !self.is_layout(c)) else {
            return;
        };
        if self.kind(lhs) != "assignment_expression" {
            return;
        }
        let children = self.children(lhs);
        let Some(at) = children.iter().position(|&child| self.kind(child) == "=") else {
            return;
        };
        let Some(target) = children[..at]
            .iter()
            .rev()
            .find(|&&child| matches!(self.kind(child), "identifier" | "member_access" | "field_access"))
            .and_then(|&child| self.last_name(child))
        else {
            return;
        };
        if target.is_empty() {
            return;
        }
        let line = self.line(id);
        let owner = self.frame().class.clone();
        self.add_symbol(target, String::new(), KIND_FUNCTION, line, Vec::new());
    }

    /// Every call inside a lambda body, block or expression, at any depth. The
    /// shared `c_calls_in_pub` stops at a block and at a nested lambda because a
    /// C/C++ lambda body is entered on its own walk; a C# lambda is not, so its
    /// block has to be read here or not at all.
    pub(crate) fn cs_initializer_vars(&mut self, id: u32) {
        let children: Vec<u32> = self
            .children(id)
            .into_iter()
            .filter(|&c| !self.is_layout(c) && !matches!(self.kind(c), "{" | "}" | ","))
            .collect();
        let mut i = 0;
        while i + 2 < children.len() {
            if self.kind(children[i + 1]) == "=" {
                if let Some(target) = self.leaf_name(children[i]) {
                    self.cs_bind_constructed(target, children[i + 2]);
                }
                i += 3;
            } else {
                i += 1;
            }
        }
    }

    pub(crate) fn lambda_calls(&self, id: u32, out: &mut Vec<u32>) {
        for child in self.children(id) {
            if matches!(
                self.kind(child),
                "invocation_expression" | "object_creation" | "object_creation_expression" | "call_expression"
                    | "generic_invocation" | "conditional_invocation"
            ) {
                out.push(child);
            }
            self.lambda_calls(child, out);
        }
    }

    pub(crate) fn erl_recovered_calls(&mut self, id: u32) -> Vec<usize> {
        let mut leaves = Vec::new();
        self.cs_collect_leaves(id, &mut leaves);
        let mut calls = Vec::new();
        let mut i = 0;
        while i + 1 < leaves.len() {
            let tok = leaves[i];
            let k = self.kind(tok);
            let is_name = matches!(
                k,
                "atom" | "variable" | "quoted_atom" | "name"
                    | "module" | "export" | "import" | "behaviour" | "include"
                    | "define" | "ifdef" | "endif" | "spec" | "type" | "record"
            );
            if is_name && self.kind(leaves[i + 1]) == "(" {
                let raw = self.text_string(tok);
                let name = if k == "quoted_atom" {
                    raw.trim_matches('\'').to_string()
                } else {
                    raw
                };
                if !name.is_empty() {
                    let line = self.line(tok);
                    calls.push(self.push_call(name, line));
                }
            }
            i += 1;
        }
        calls
    }

    pub(crate) fn erl_leaf_name(&self, tok: u32) -> Option<String> {
        let k = self.kind(tok);
        if matches!(
            k,
            "atom" | "variable" | "quoted_atom" | "name"
                | "module" | "export" | "import" | "behaviour" | "include"
                | "define" | "ifdef" | "endif" | "spec" | "type" | "record"
                | "true" | "false"
        ) {
            let raw = self.text_string(tok);
            let name = if k == "quoted_atom" {
                raw.trim_matches('\'').to_string()
            } else {
                raw
            };
            if !name.is_empty() {
                return Some(name);
            }
        }
        None
    }

    fn erl_chunk_form(&mut self, chunk: &[u32]) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut i = 0;
        while i < chunk.len() {
            let tok = chunk[i];
            if let Some(name) = self.erl_leaf_name(tok) {
                if i + 1 < chunk.len() && self.kind(chunk[i + 1]) == "(" {
                    let line = self.line(tok);
                    let idx = self.push_call(name.clone(), line);
                    parts.push(format!("[\"c\",{idx},0,[0]]"));
                }
                let mut f = String::from("[\"n\",");
                push_string(&mut f, &name);
                f.push(']');
                parts.push(f);
            }
            i += 1;
        }
        match parts.len() {
            0 => "0".to_string(),
            1 => parts.remove(0),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        }
    }

    fn erl_emit_recovered_chunk(&mut self, chunk: &[u32]) {
        if chunk.is_empty() {
            return;
        }
        let line = self.line(chunk[0]);
        let mut depth = 0i32;
        let mut eq_at = None;
        for (i, &tok) in chunk.iter().enumerate() {
            match self.kind(tok) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => depth = (depth - 1).max(0),
                "=" | "=:=" if depth == 0 => {
                    eq_at = Some(i);
                    break;
                }
                _ => {}
            }
        }
        if let Some(pos) = eq_at {
            let lhs = &chunk[..pos];
            let rhs = &chunk[pos + 1..];
            let mut targets: Vec<String> = Vec::new();
            for &tok in lhs {
                if let Some(name) = self.erl_leaf_name(tok) {
                    if !targets.contains(&name) {
                        targets.push(name);
                    }
                }
            }
            let rhs_val = self.erl_chunk_form(rhs);
            if !targets.is_empty() {
                self.lexer_event(b'a', line, &targets, &rhs_val, false);
            }
            let full_val = self.erl_chunk_form(chunk);
            if full_val != "0" {
                self.lexer_event(b'x', line, &[], &full_val, false);
            }
        } else {
            let full_val = self.erl_chunk_form(chunk);
            if full_val != "0" {
                self.lexer_event(b'x', line, &[], &full_val, false);
            }
        }
    }

    pub(crate) fn erl_recovered_events(&mut self, id: u32) {
        let mut leaves = Vec::new();
        self.cs_collect_leaves(id, &mut leaves);
        let mut chunk: Vec<u32> = Vec::new();
        let mut depth = 0i32;
        for tok in leaves {
            let k = self.kind(tok);
            match k {
                "(" | "[" | "{" => {
                    depth += 1;
                    chunk.push(tok);
                }
                ")" | "]" | "}" => {
                    depth = (depth - 1).max(0);
                    chunk.push(tok);
                }
                "," | ";" | "." | "->" if depth == 0 => {
                    self.erl_emit_recovered_chunk(&chunk);
                    chunk.clear();
                }
                _ => {
                    chunk.push(tok);
                }
            }
        }
        self.erl_emit_recovered_chunk(&chunk);
    }

    pub(crate) fn cs_has_error(&self, id: u32) -> bool {
        if self.kind(id) == "ERROR" {
            return true;
        }
        self.children(id).into_iter().any(|c| self.cs_has_error(c))
    }

    pub(crate) fn cs_collect_leaves(&self, id: u32, out: &mut Vec<u32>) {
        let node = self.node(id);
        if node.first_child == NONE {
            if node.start < node.end && !self.kind(id).contains("comment") {
                out.push(id);
            }
            return;
        }
        for child in self.children(id) {
            self.cs_collect_leaves(child, out);
        }
    }

    pub(crate) fn cs_recovered_calls(&mut self, id: u32) {
        let tokens: Vec<u32> = self.children(id);
        self.cs_recovered_calls_tokens(&tokens);
    }

    pub(crate) fn cs_recovered_calls_tokens(&mut self, tokens: &[u32]) {
        let mut at = 0;
        while at < tokens.len() {
            if matches!(
                self.kind(tokens[at]),
                "member_access_expression" | "member_access" | "conditional_member_access"
            ) {
                let name = self.path_of(tokens[at]);
                if !name.is_empty()
                    && at + 1 < tokens.len()
                    && matches!(self.kind(tokens[at + 1]), "(" | "argument_list" | "arguments")
                {
                    let line = self.line(tokens[at]);
                    self.push_call(name, line);
                }
                at += 1;
                continue;
            }
            let Some(mut name) = self.leaf_name(tokens[at]) else {
                at += 1;
                continue;
            };
            if matches!(
                name.as_str(),
                "if" | "for" | "foreach" | "while" | "switch" | "catch" | "using" | "lock" | "fixed"
                    | "checked" | "unchecked" | "sizeof" | "typeof" | "default" | "return" | "throw"
                    | "yield" | "await" | "new" | "class" | "struct" | "interface" | "enum" | "record"
                    | "delegate" | "namespace" | "where" | "when" | "is" | "as" | "in" | "out" | "ref"
            ) {
                at += 1;
                continue;
            }
            if at > 0
                && matches!(
                    self.kind(tokens[at - 1]),
                    "class" | "struct" | "interface" | "enum" | "record" | "namespace"
                )
            {
                at += 1;
                continue;
            }
            let is_new = at > 0 && self.kind(tokens[at - 1]) == "new";
            if name == "this" {
                name = "self".to_string();
            }
            let mut end = at;
            let mut line = self.line(tokens[at]);
            while end + 2 < tokens.len() && matches!(self.kind(tokens[end + 1]), "." | "?." | "::") {
                let Some(seg) = self.leaf_name(tokens[end + 2]) else {
                    break;
                };
                name.push('.');
                name.push_str(&seg);
                line = self.line(tokens[end + 2]);
                end += 2;
            }
            let mut after = end + 1;
            if after < tokens.len() && self.kind(tokens[after]) == "<" {
                let mut depth = 0i32;
                let mut j = after;
                while j < tokens.len() {
                    match self.kind(tokens[j]) {
                        "<" => depth += 1,
                        ">" | ">joined" => {
                            depth -= 1;
                            if depth <= 0 {
                                after = j + 1;
                                break;
                            }
                        }
                        ";" | "{" | "}" => break,
                        _ => {}
                    }
                    j += 1;
                }
            }
            if after < tokens.len() {
                let next_kind = self.kind(tokens[after]);
                if matches!(next_kind, "(" | "argument_list" | "arguments")
                    || (is_new && next_kind == "{")
                {
                    self.push_call(name.clone(), line);
                    let last = name.rsplit('.').next().unwrap_or("");
                    if last.starts_with(|ch: char| ch.is_uppercase()) {
                        let mut scan = at;
                        if is_new && scan > 0 {
                            scan -= 1;
                        }
                        if scan > 1 && self.kind(tokens[scan - 1]) == "=" {
                            let mut t_end = scan - 2;
                            if self.kind(tokens[t_end]) == "]" {
                                let mut depth = 0usize;
                                while t_end > 0 {
                                    match self.kind(tokens[t_end]) {
                                        "]" => depth += 1,
                                        "[" => {
                                            depth -= 1;
                                            if depth == 0 {
                                                break;
                                            }
                                        }
                                        _ => {}
                                    }
                                    t_end -= 1;
                                }
                                if depth != 0 || t_end == 0 {
                                    at = end + 1;
                                    continue;
                                }
                                t_end -= 1;
                            }
                            if let Some(mut target) = self.leaf_name(tokens[t_end]) {
                                while t_end >= 2 && matches!(self.kind(tokens[t_end - 1]), "." | "?.") {
                                    if let Some(prev) = self.leaf_name(tokens[t_end - 2]) {
                                        let prev = if prev == "this" { "self".to_string() } else { prev };
                                        target = format!("{prev}.{target}");
                                        t_end -= 2;
                                    } else {
                                        break;
                                    }
                                }
                                if !target.is_empty() && !target.contains('.') {
                                    self.facts.vars.push((target, last.to_string()));
                                }
                            }
                        }
                    }
                }
            }
            at = end + 1;
        }
    }

    /// Recover statement events (`x`, `a`, `r`) from flat tokens inside a C#
    /// `ERROR` node. `cs_recovered_calls` already recovers calls and `vars`
    /// bindings, but emitted no events, leaving every statement inside an
    /// unparsed property/accessor/method body without an event on its line.
    pub(crate) fn cs_recovered_events(&mut self, id: u32) {
        let children: Vec<u32> = self
            .children(id)
            .into_iter()
            .filter(|&c| {
                let node = self.node(c);
                node.start < node.end && !self.kind(c).contains("comment")
            })
            .collect();
        let mut stmt: Vec<u32> = Vec::new();
        let mut paren_depth = 0i32;
        let mut brace_depth = 0i32;

        for (idx, &child) in children.iter().enumerate() {
            let kind = self.kind(child);
            if Self::is_cs_handled_node(kind) {
                self.cs_emit_recovered_stmt(&stmt);
                stmt.clear();
                paren_depth = 0;
                brace_depth = 0;
                continue;
            }
            match kind {
                "(" | "[" => {
                    paren_depth += 1;
                    stmt.push(child);
                }
                ")" | "]" => {
                    paren_depth = (paren_depth - 1).max(0);
                    stmt.push(child);
                    if kind == ")" && paren_depth == 0 && brace_depth == 0 {
                        if let Some(&first) = stmt.first() {
                            if matches!(
                                self.kind(first),
                                "if" | "while" | "for" | "foreach" | "using" | "lock" | "fixed" | "catch"
                            ) {
                                let next_kind = children.get(idx + 1).map(|&n| self.kind(n)).unwrap_or("");
                                if !matches!(next_kind, "{" | ";") {
                                    self.cs_emit_recovered_stmt(&stmt);
                                    stmt.clear();
                                }
                            }
                        }
                    }
                }
                "{" => {
                    if paren_depth > 0 || brace_depth > 0 || self.cs_is_expr_brace(&stmt) {
                        brace_depth += 1;
                        stmt.push(child);
                    } else {
                        self.cs_emit_recovered_stmt(&stmt);
                        stmt.clear();
                    }
                }
                "}" => {
                    if brace_depth > 0 {
                        brace_depth -= 1;
                        stmt.push(child);
                    } else {
                        self.cs_emit_recovered_stmt(&stmt);
                        stmt.clear();
                    }
                }
                ";" if paren_depth == 0 && brace_depth == 0 => {
                    self.cs_emit_recovered_stmt(&stmt);
                    stmt.clear();
                }
                "=>" if paren_depth == 0 && brace_depth == 0 => {
                    if self.cs_is_accessor_tokens(&stmt) {
                        self.cs_emit_recovered_stmt(&stmt);
                    }
                    stmt.clear();
                }
                "else" if paren_depth == 0 && brace_depth == 0 && stmt.is_empty() => {}
                _ => {
                    stmt.push(child);
                }
            }
        }
        self.cs_emit_recovered_stmt(&stmt);
    }

    fn is_cs_handled_node(kind: &str) -> bool {
        matches!(
            kind,
            "class_declaration"
                | "struct_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "record_declaration"
                | "method_declaration"
                | "constructor_declaration"
                | "delegate_declaration"
                | "local_function_statement"
                | "field_declaration"
                | "event_field_declaration"
                | "event_declaration"
                | "property_declaration"
                | "indexer_declaration"
                | "operator_declaration"
                | "conversion_operator_declaration"
                | "accessor_declaration"
                | "expression_bodied_member"
                | "local_variable_declaration"
                | "using_declaration"
                | "expression_statement"
                | "return_statement"
                | "throw_statement"
                | "yield_statement"
                | "break_statement"
                | "continue_statement"
                | "if_statement"
                | "for_statement"
                | "foreach_statement"
                | "while_statement"
                | "do_statement"
                | "switch_statement"
                | "try_statement"
                | "catch_clause"
                | "finally_clause"
                | "using_statement"
                | "lock_statement"
                | "fixed_statement"
                | "block"
                | "ERROR"
        )
    }

    fn cs_is_expr_brace(&self, stmt: &[u32]) -> bool {
        let Some(&last) = stmt.last() else {
            return false;
        };
        if matches!(self.kind(last), "=" | "new" | "is" | "=>" | "," | "return" | "yield") {
            return true;
        }
        stmt.iter().any(|&n| self.kind(n) == "new") && matches!(self.kind(last), ")" | "]" | ">" | "identifier")
    }

    fn cs_is_accessor_tokens(&self, stmt: &[u32]) -> bool {
        let Some(&last) = stmt.last() else {
            return false;
        };
        if !matches!(self.kind(last), "get" | "set" | "init" | "add" | "remove") {
            return false;
        }
        stmt[..stmt.len() - 1]
            .iter()
            .all(|&n| matches!(self.kind(n), "public" | "private" | "protected" | "internal"))
    }

    fn cs_emit_recovered_stmt(&mut self, stmt: &[u32]) {
        if stmt.is_empty() {
            return;
        }
        let line = self.line(stmt[0]);
        if self.cs_is_accessor_tokens(stmt) {
            let mut parts: Vec<String> = Vec::new();
            for &n in stmt {
                let mut f = String::from("[\"n\",");
                push_string(&mut f, self.kind(n));
                f.push(']');
                parts.push(f);
            }
            let value = if parts.len() == 1 {
                parts.pop().unwrap()
            } else {
                format!("[\"m\",[{}]]", parts.join(","))
            };
            self.lexer_event(b'x', line, &[], &value, false);
            return;
        }
        let first_kind = self.kind(stmt[0]);
        if first_kind == "return" {
            let value = self.cs_recovered_form(&stmt[1..]);
            self.lexer_event(b'r', line, &[], &value, false);
            return;
        }
        if matches!(
            first_kind,
            "if" | "while" | "for" | "foreach" | "switch" | "catch" | "using" | "lock" | "fixed"
        ) {
            let rest = &stmt[1..];
            let inner = if rest.first().is_some_and(|&n| self.kind(n) == "(")
                && rest.last().is_some_and(|&n| self.kind(n) == ")")
                && rest.len() >= 2
            {
                &rest[1..rest.len() - 1]
            } else {
                rest
            };
            let value = self.cs_recovered_form(inner);
            if value != "0" && !value.is_empty() {
                self.lexer_event(b'x', line, &[], &value, false);
            }
            return;
        }
