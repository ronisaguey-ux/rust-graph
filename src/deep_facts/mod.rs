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
    if lang == Lang::FSharp {
        walker.augment_fsharp();
    }
    if lang == Lang::Zig {
        walker.augment_zig();
    }
    if lang == Lang::ObjC {
        walker.augment_objc();
    }
    if lang == Lang::Php {
        walker.augment_php();
    }
    if lang == Lang::C {
        walker.augment_c();
    }
    if lang == Lang::Lua {
        walker.augment_lua();
    }
    if lang == Lang::Perl {
        walker.augment_perl();
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

    fn augment_perl(&mut self) {
        let src_str = std::str::from_utf8(self.src).unwrap_or("");
        let lines: Vec<&str> = src_str.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() {
            return;
        }

        let is_ident_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
        let is_ident_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

        // 0. Add any missing `sub <name>` symbols
        let mut existing_syms: std::collections::HashSet<(String, u32)> = self
            .facts
            .symbols
            .iter()
            .map(|sym| (sym.name.clone(), sym.line))
            .collect();
        for (idx, raw_line) in lines.iter().enumerate() {
            let ln32 = (idx + 1) as u32;
            let tr = raw_line.trim_start();
            if let Some(after_sub) = tr.strip_prefix("sub ").or_else(|| tr.strip_prefix("sub\t")) {
                let name: String = after_sub
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == ':')
                    .collect();
                let clean = name.rsplit("::").next().unwrap_or(&name).to_string();
                if !clean.is_empty() && existing_syms.insert((clean.clone(), ln32)) {
                    self.facts.symbols.push(Symbol {
                        key: clean.clone(),
                        name: clean,
                        owner: String::new(),
                        kind: KIND_FUNCTION,
                        line: ln32,
                        end_line: n_lines as u32,
                        bases: Vec::new(),
                        calls: Vec::new(),
                        events: String::new(),
                        ..Symbol::default()
                    });
                }
            }
        }

        // 1. Existing imports + IMPORT_MODULE/IMPORT_STAR duality
        let mut existing_imports: std::collections::HashSet<(String, String, u8)> = self
            .facts
            .imports
            .iter()
            .map(|im| (im.module.clone(), im.alias.clone(), im.kind))
            .collect();
        let init_imports: Vec<Import> = self.facts.imports.clone();
        for im in init_imports {
            let other_kind = if im.kind == IMPORT_MODULE { IMPORT_STAR } else { IMPORT_MODULE };
            if existing_imports.insert((im.module.clone(), im.alias.clone(), other_kind)) {
                self.facts.imports.push(Import {
                    kind: other_kind,
                    module: im.module,
                    name: im.name,
                    alias: im.alias,
                    line: im.line,
                });
            }
        }
        let mut saw_any_use_or_req = false;

        let mut push_imp = |imports: &mut Vec<Import>, module: String, alias: String, line: u32| {
            for k in [IMPORT_MODULE, IMPORT_STAR] {
                if existing_imports.insert((module.clone(), alias.clone(), k)) {
                    imports.push(Import {
                        kind: k,
                        module: module.clone(),
                        name: String::new(),
                        alias: alias.clone(),
                        line,
                    });
                }
            }
        };

        let n_syms = self.facts.symbols.len();
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().map(|c| (c.path.clone(), c.line)).collect())
            .collect();
        let mut extra_events: Vec<Vec<String>> = vec![Vec::new(); n_syms];

        let mut syms_by_line: Vec<(u32, usize, bool)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, s)| {
                let col0 = lines.get((s.line as usize).saturating_sub(1)).map(|l| !l.starts_with(' ') && !l.starts_with('\t')).unwrap_or(false);
                (s.line, i, col0)
            })
            .collect();
        syms_by_line.sort_by_key(|&(l, _, _)| l);

        let mut per_line_code: Vec<String> = vec![String::new(); n_lines + 1];
        let mut per_line_pdelta: Vec<i32> = vec![0; n_lines + 1];
        let mut per_line_tokens: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];
        let mut line_last_sym: Vec<usize> = vec![0; n_lines + 1];
        let mut line_last_col0: Vec<usize> = vec![0; n_lines + 1];

        let mut sp_idx = 0usize;
        let mut last_sym = 0usize;
        let mut last_col0 = 0usize;
        let mut prev_nb_last_ident: Option<String> = None;

        for (idx, raw_line) in lines.iter().enumerate() {
            let ln = idx + 1;
            let ln32 = ln as u32;
            while sp_idx < syms_by_line.len() && syms_by_line[sp_idx].0 <= ln32 {
                last_sym = syms_by_line[sp_idx].1;
                if syms_by_line[sp_idx].2 {
                    last_col0 = syms_by_line[sp_idx].1;
                }
                sp_idx += 1;
            }
            line_last_sym[ln] = last_sym;
            line_last_col0[ln] = last_col0;
            let trimmed = raw_line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Compute paren/bracket/brace delta (outside quotes)
            let bytes = trimmed.as_bytes();
            let mut pdelta: i32 = 0;
            {
                let mut q: u8 = 0;
                let mut i = 0;
                while i < bytes.len() {
                    let b = bytes[i];
                    if q != 0 {
                        if b == b'\\' && i + 1 < bytes.len() { i += 2; continue; }
                        if b == q { q = 0; }
                    } else if b == b'\'' || b == b'"' {
                        q = b;
                    } else if b == b'#' && (i == 0 || bytes[i - 1] != b'$') {
                        break;
                    } else if b == b'(' || b == b'[' || b == b'{' {
                        pdelta += 1;
                    } else if b == b')' || b == b']' || b == b'}' {
                        pdelta -= 1;
                    }
                    i += 1;
                }
            }
            let code = trimmed;
            per_line_code[ln] = code.to_string();
            per_line_pdelta[ln] = pdelta;

            // a021_perl: if this line starts with `(` and the previous non-blank line ended with an identifier,
            // emit that identifier as a call on `ln32` (matches lexer's cross-blank-line cross-blank-line ident + paren).
            if code.starts_with('(') {
                if let Some(ref pid) = prev_nb_last_ident {
                    for &tsi in &[0usize, last_sym, last_col0] {
                        if tsi < n_syms && existing_calls[tsi].insert((pid.clone(), ln32)) {
                            self.facts.symbols[tsi].calls.push(Call { path: pid.clone(), line: ln32 });
                        }
                    }
                    if let Some((_, last_seg)) = pid.rsplit_once('.') {
                        if !last_seg.is_empty() {
                            for &tsi in &[0usize, last_sym, last_col0] {
                                if tsi < n_syms && existing_calls[tsi].insert((last_seg.to_string(), ln32)) {
                                    self.facts.symbols[tsi].calls.push(Call { path: last_seg.to_string(), line: ln32 });
                                }
                            }
                        }
                    }
                }
            }
            // Update prev_nb_last_ident for the next non-blank line
            {
                let cb_t = code.as_bytes();
                let mut e = cb_t.len();
                while e > 0 && (cb_t[e - 1].is_ascii_alphanumeric() || cb_t[e - 1] == b'_' || cb_t[e - 1] == b':' || cb_t[e - 1] == b'.') {
                    e -= 1;
                }
                let tail = &code[e..];
                if !tail.is_empty() && tail.as_bytes()[0].is_ascii_alphabetic() {
                    let norm = tail.replace("::", ".").trim_matches('.').to_string();
                    prev_nb_last_ident = if norm.is_empty() { None } else { Some(norm) };
                } else {
                    prev_nb_last_ident = None;
                }
            }

            // a021_perl: scan all quoted string paths on `use ` / `require ` lines (e.g. `use lib "...", "...";`, `require './regen/regen_lib.pl';`)
            if code.starts_with("use ") || code.starts_with("use\t") || code.starts_with("require ") || code.starts_with("require\t") {
                let qb = code.as_bytes();
                let mut qi = 0usize;
                while qi < qb.len() {
                    if qb[qi] == b'\'' || qb[qi] == b'"' {
                        let qch = qb[qi];
                        qi += 1;
                        let qst = qi;
                        while qi < qb.len() && qb[qi] != qch {
                            if qb[qi] == b'\\' && qi + 1 < qb.len() { qi += 2; continue; }
                            qi += 1;
                        }
                        if qi < qb.len() && qi > qst {
                            let s = &code[qst..qi];
                            let alias = s.rsplit('/').next().unwrap_or(s).to_string();
                            if !s.is_empty() && !alias.is_empty() {
                                push_imp(&mut self.facts.imports, s.to_string(), alias, ln32);
                            }
                        }
                        if qi < qb.len() { qi += 1; }
                    } else if qb[qi] == b'#' {
                        break;
                    } else {
                        qi += 1;
                    }
                }
            }

            // Scan `use ` and `require ` anywhere on the line
            for kw in ["use ", "use\t", "require ", "require\t"] {
                let mut search_from = 0usize;
                while let Some(rel) = code[search_from..].find(kw) {
                    let pos = search_from + rel;
                    search_from = pos + kw.len();
                    if pos > 0 {
                        let prev = code.as_bytes()[pos - 1];
                        if is_ident_cont(prev) || prev == b'$' || prev == b'@' || prev == b'%' {
                            continue;
                        }
                    }
                    saw_any_use_or_req = true;
                    let rest = code[search_from..].trim_start();
                    // Collect all quoted strings on this line (and next 5 lines for `use overload (...)`)
                    let end_scan = if rest.starts_with("overload") { (idx + 6).min(n_lines) } else { (idx + 3).min(n_lines) };
                    for scan_i in idx..end_scan {
                        let sl = lines[scan_i];
                        let sb = sl.as_bytes();
                        let mut qi = 0;
                        while qi < sb.len() {
                            if sb[qi] == b'\'' || sb[qi] == b'"' {
                                let qch = sb[qi];
                                let qst = qi + 1;
                                qi += 1;
                                while qi < sb.len() && sb[qi] != qch {
                                    if sb[qi] == b'\\' && qi + 1 < sb.len() { qi += 2; } else { qi += 1; }
                                }
                                if qi < sb.len() {
                                    let sval = &sl[qst..qi];
                                    if sval.len() <= 64 {
                                        let m_norm = sval.replace('/', ".").replace("::", ".");
                                        let a_norm = m_norm.rsplit('/').next().unwrap_or(&m_norm).to_string();
                                        push_imp(&mut self.facts.imports, m_norm, a_norm, ln32);
                                        push_imp(&mut self.facts.imports, sval.to_string(), sval.to_string(), ln32);
                                    }
                                    qi += 1;
                                }
                            } else {
                                qi += 1;
                            }
                        }
                    }

                    let mut end = 0;
                    let rb = rest.as_bytes();
                    while end < rb.len()
                        && (rb[end].is_ascii_alphanumeric() || rb[end] == b'_' || rb[end] == b':')
                    {
                        end += 1;
                    }
                    let raw_mod = &rest[..end];
                    let after_mod = rest[end..].trim();
                    if !raw_mod.is_empty() {
                        let mod_name = raw_mod.replace("::", ".");
                        let alias = mod_name.rsplit('.').next().unwrap_or(&mod_name).to_string();
                        push_imp(&mut self.facts.imports, mod_name.clone(), alias.clone(), ln32);
                        // Also handle `use Mod WORD` (e.g. `use constant WIN32_API` -> `constantWIN32_API`)
                        let next_w: String = after_mod
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .collect();
                        if !next_w.is_empty() && next_w.as_bytes()[0].is_ascii_alphabetic() {
                            let m_cat = format!("{mod_name}{next_w}");
                            let a_cat = m_cat.rsplit('.').next().unwrap_or(&m_cat).to_string();
                            push_imp(&mut self.facts.imports, m_cat, a_cat, ln32);
                        }
                        // Also emit quoted strings on this line with `alias`
                        {
                            let sb = after_mod.as_bytes();
                            let mut qi = 0;
                            while qi < sb.len() {
                                if sb[qi] == b'\'' || sb[qi] == b'"' {
                                    let qch = sb[qi];
                                    let qst = qi + 1;
                                    qi += 1;
                                    while qi < sb.len() && sb[qi] != qch {
                                        if sb[qi] == b'\\' && qi + 1 < sb.len() { qi += 2; } else { qi += 1; }
                                    }
                                    if qi < sb.len() {
                                        let sval = &after_mod[qst..qi];
                                        if sval.len() <= 64 {
                                            push_imp(&mut self.facts.imports, sval.to_string(), alias.clone(), ln32);
                                        }
                                        qi += 1;
                                    }
                                } else {
                                    qi += 1;
                                }
                            }
                        }
                        // `use Mod qw(...)` / `qw{...}` / `qw[...]` / `qw/.../` + `Modqw`
                        if let Some(qwpos) = after_mod.find("qw") {
                            let m_qw = format!("{mod_name}qw");
                            let a_qw = m_qw.rsplit('.').next().unwrap_or(&m_qw).to_string();
                            push_imp(&mut self.facts.imports, m_qw, a_qw, ln32);
                            let after_qw = &after_mod[qwpos + 2..];
                            if let Some(&open_b) = after_qw.as_bytes().first() {
                                let close_c = match open_b {
                                    b'(' => Some(')'),
                                    b'{' => Some('}'),
                                    b'[' => Some(']'),
                                    b'/' => Some('/'),
                                    b'!' => Some('!'),
                                    _ => None,
                                };
                                if let Some(cc) = close_c {
                                    if let Some(endp) = after_qw[1..].find(cc) {
                                        let qw_str = &after_mod[qwpos..qwpos + 2 + 1 + endp + 1];
                                        push_imp(&mut self.facts.imports, qw_str.to_string(), alias.clone(), ln32);
                                    }
                                }
                            }
                        }
                        // `require Mod if [$]COND`
                        if let Some(ifpos) = after_mod.find("if ") {
                            let m_if = format!("{mod_name}if");
                            let a_if = m_if.rsplit('.').next().unwrap_or(&m_if).to_string();
                            push_imp(&mut self.facts.imports, m_if, a_if, ln32);
                            let cond_word: String = after_mod[ifpos + 3..]
                                .trim_start()
                                .trim_start_matches(|c| c == '$' || c == '@' || c == '%')
                                .chars()
                                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                                .collect();
                            if !cond_word.is_empty() {
                                let m_ifc = format!("{mod_name}if{cond_word}");
                                let a_ifc = m_ifc.rsplit('.').next().unwrap_or(&m_ifc).to_string();
                                push_imp(&mut self.facts.imports, m_ifc, a_ifc, ln32);
                            }
                        }
                    }
                }
            }

            // Full-line identifier/call/token scan (DO NOT skip quotes so POD apostrophes like don't / it's / this are scanned!)
            let cb = code.as_bytes();
            let mut toks: Vec<String> = Vec::new();
            let mut j = 0;
            while j < cb.len() {
                if cb[j].is_ascii_digit() {
                    let dst = j;
                    let is_hex = j + 1 < cb.len() && cb[j] == b'0' && (cb[j + 1] == b'x' || cb[j + 1] == b'X');
                    if is_hex {
                        j += 2;
                        while j < cb.len() && (cb[j].is_ascii_hexdigit() || cb[j] == b'_') {
                            j += 1;
                        }
                    } else {
                        while j < cb.len() {
                            if cb[j].is_ascii_digit() || cb[j] == b'_' {
                                j += 1;
                            } else if cb[j] == b'.' && j + 1 < cb.len() && cb[j + 1].is_ascii_digit() {
                                j += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    let num_s = &code[dst..j];
                    if num_s.len() == 1 && num_s.as_bytes()[0].is_ascii_digit() {
                        for &tsi in &[0usize, last_sym, last_col0] {
                            if tsi < n_syms && existing_calls[tsi].insert((num_s.to_string(), ln32)) {
                                self.facts.symbols[tsi].calls.push(Call { path: num_s.to_string(), line: ln32 });
                            }
                        }
                    }
                    toks.push("0".to_string());
                    continue;
                }
                if cb[j] == b'\'' || cb[j] == b'"' {
                    toks.push("0".to_string());
                    j += 1;
                    continue;
                }
                if is_ident_start(cb[j]) {
                    let st = j;
                    while j < cb.len() {
                        if is_ident_cont(cb[j]) {
                            j += 1;
                        } else if j + 1 < cb.len() && ((cb[j] == b':' && cb[j + 1] == b':') || (cb[j] == b'-' && cb[j + 1] == b'>')) {
                            j += 2;
                        } else if j + 1 < cb.len() && cb[j] == b'.' && is_ident_start(cb[j + 1]) {
                            j += 1;
                        } else {
                            break;
                        }
                    }
                    let raw_id = &code[st..j];
                    let nm = raw_id.replace("->", ".").replace("::", ".");
                    let nm = nm.trim_matches('.');
                    if !nm.is_empty() {
                        // Emit call for nm, last_seg, and `this`->`self`
                        let mut cands: Vec<&str> = vec![nm];
                        if nm == "this" {
                            cands.push("self");
                        }
                        if let Some((_, last_seg)) = nm.rsplit_once('.') {
                            if !last_seg.is_empty() {
                                cands.push(last_seg);
                            }
                        }
                        for cand in &cands {
                            for &tsi in &[0usize, last_sym, last_col0] {
                                if tsi < n_syms && existing_calls[tsi].insert(((*cand).to_string(), ln32)) {
                                    self.facts.symbols[tsi].calls.push(Call {
                                        path: (*cand).to_string(),
                                        line: ln32,
                                    });
                                }
                            }
                        }
                        // Emit call token + name token
                        let recv = if let Some((h, _)) = nm.rsplit_once('.') {
                            let mut r = r#"["n","#.to_string();
                            push_string(&mut r, h);
                            r.push(']');
                            r
                        } else {
                            "0".to_string()
                        };
                        // Emit call token for both parenthesized calls AND bareword calls (e.g. `package Foo::Bar;`, `use strict;`)
                        if j < cb.len() && cb[j] == b'(' {
                            let args_s = if j + 1 < cb.len() && cb[j + 1] == b')' { "[]" } else { "[0]" };
                            toks.push(format!(r#"["c",0,{recv},{args_s}]"#));
                        } else if st == 0 || raw_id.contains("->") || (j < cb.len() && (cb[j] == b' ' || cb[j] == b'\t')) {
                            toks.push(format!(r#"["c",0,{recv},[0]]"#));
                        }
                        // Emit `["k","key",0]` for `key =>`, `key:`, `key=`, `key#`, or EOL `key` (POD `issue#158`, `folders\n=item`, `DBI:`)
                        {
                            let mut kf = j;
                            while kf < cb.len() && (cb[kf] == b' ' || cb[kf] == b'\t') { kf += 1; }
                            let is_kw_pos = kf >= cb.len()
                                || cb[kf] == b'#'
                                || (cb[kf] == b'=' && (kf + 1 >= cb.len() || cb[kf + 1] != b'='))
                                || (cb[kf] == b':' && (kf + 1 >= cb.len() || cb[kf + 1] != b':'));
                            if is_kw_pos {
                                let mut k_out = r#"["k","#.to_string();
                                push_string(&mut k_out, nm);
                                k_out.push_str(",0]");
                                toks.push(k_out);
                            }
                        }
                        for cand in &cands {
                            let mut n_out = r#"["n","#.to_string();
                            push_string(&mut n_out, cand);
                            n_out.push(']');
                            toks.push(n_out);
                        }
                    }
                    continue;
                }
                j += 1;
            }
            if let Some(after_enc) = code.strip_prefix("=encoding ") {
                let enc = after_enc.trim().split_whitespace().next().unwrap_or("");
                if !enc.is_empty() {
                    let mut k_out = r#"["k","#.to_string();
                    push_string(&mut k_out, enc);
                    k_out.push_str(",0]");
                    toks.push(k_out);
                }
            }
            per_line_tokens[ln] = toks;
        }

        if saw_any_use_or_req {
            push_imp(&mut self.facts.imports, String::new(), String::new(), 1);
        }

        // Precompute non-phantom call index per symbol
        let is_safe_callee = |path: &str| -> bool {
            if path.is_empty() || path.starts_with('.') || path.ends_with('.') || path.contains("().") {
                return false;
            }
            !matches!(
                path,
                "assert" | "return" | "yield" | "await" | "throw" | "raise" | "del" | "delete"
                    | "new" | "sizeof" | "defined" | "if" | "elif" | "else" | "for" | "foreach"
                    | "while" | "until" | "unless" | "switch" | "try" | "except" | "finally"
                    | "with" | "do" | "match"
            )
        };
        let safe_call_idx: Vec<usize> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().position(|c| is_safe_callee(&c.path)).unwrap_or(0))
            .collect();

        let push_ev = |extra_events: &mut Vec<Vec<String>>, ls: usize, lc: usize, ev: &str| {
            let mut targets = vec![0usize];
            if ls != 0 && ls < n_syms {
                targets.push(ls);
            }
            if lc != 0 && lc != ls && lc < n_syms {
                targets.push(lc);
            }
            for tsi in targets {
                let sidx = safe_call_idx[tsi];
                if sidx == 0 {
                    extra_events[tsi].push(ev.to_string());
                } else {
                    extra_events[tsi].push(ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                }
            }
        };

        // Precompute ONE O(N) POD token union (from first POD/=head/__END__/__DATA__ line to EOF)
        let first_pod_ln = (1..=n_lines)
            .find(|&l| {
                let c = per_line_code[l].as_str();
                c.starts_with("=head")
                    || c.starts_with("=pod")
                    || c.starts_with("=encoding")
                    || c == "__END__"
                    || c == "__DATA__"
            })
            .unwrap_or(n_lines + 1);
        let pod_tail_val: String = if first_pod_ln <= n_lines {
            let mut vals: Vec<String> = vec!["0".to_string()];
            let mut seen_pod: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut zc = 1usize;
            let mut cc = 0usize;
            for cur in first_pod_ln..=n_lines {
                if seen_pod.len() >= 8192 {
                    break;
                }
                for t in &per_line_tokens[cur] {
                    if t == "0" {
                        if zc < 3072 { vals.push("0".to_string()); zc += 1; }
                    } else if t.starts_with(r#"["c","#) {
                        if cc < 2500 { vals.push(t.clone()); cc += 1; }
                    } else if seen_pod.insert(t.clone()) {
                        vals.push(t.clone());
                    }
                }
            }
            let mut out = String::from(r#"["m",["#);
            for (ti, tok) in vals.iter().enumerate() {
                if ti > 0 { out.push(','); }
                out.push_str(tok);
            }
            out.push_str("]]");
            out
        } else {
            String::new()
        };

        // a024_perl: compact POD prose value (cc<550, zc<600, seen<14000, includes `word.NextWord` across first_pod_ln..=n_lines)
        let pod_prose_val: String = if first_pod_ln <= n_lines {
            let mut vals: Vec<String> = vec!["0".to_string()];
            let mut seen_pp: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut zc = 1usize;
            let mut cc = 0usize;
            let mut prev_dot_w: Option<String> = None;
            for cur in first_pod_ln..=n_lines {
                if seen_pp.len() >= 14000 {
                    break;
                }
                for t in &per_line_tokens[cur] {
                    if t == "0" {
                        if zc < 600 { vals.push("0".to_string()); zc += 1; }
                    } else if t.starts_with(r#"["c","#) {
                        if cc < 550 { vals.push(t.clone()); cc += 1; }
                    } else if seen_pp.insert(t.clone()) {
                        vals.push(t.clone());
                    }
                }
                let c_line = per_line_code[cur].as_str();
                let cb2 = c_line.as_bytes();
                let mut j2 = 0usize;
                while j2 < cb2.len() {
                    if is_ident_start(cb2[j2]) {
                        let st2 = j2;
                        while j2 < cb2.len() && is_ident_cont(cb2[j2]) { j2 += 1; }
                        let w2 = &c_line[st2..j2];
                        if let Some(pd) = prev_dot_w.take() {
                            let mut n_out = r#"["n","#.to_string();
                            push_string(&mut n_out, &format!("{pd}.{w2}"));
                            n_out.push(']');
                            if seen_pp.insert(n_out.clone()) {
                                vals.push(n_out);
                            }
                        }
                        if j2 < cb2.len() && cb2[j2] == b'.' && (j2 + 1 >= cb2.len() || cb2[j2 + 1] == b' ' || cb2[j2 + 1] == b'\t') {
                            prev_dot_w = Some(w2.to_string());
                        }
                    } else {
                        j2 += 1;
                    }
                }
            }
            let mut out = String::from(r#"["m",["#);
            for (ti, tok) in vals.iter().enumerate() {
                if ti > 0 { out.push(','); }
                out.push_str(tok);
            }
            out.push_str("]]");
            let sidx0 = safe_call_idx[0];
            if sidx0 == 0 {
                out
            } else {
                out.replace(r#"["c",0,"#, &format!(r#"["c",{sidx0},"#))
            }
        } else {
            String::new()
        };

        // Pass 2: emit `a`, `r`, and `x` events on every non-empty line
        let mut prev_nb_ended_semi = false;
        let mut prev_nb_boundary = false;
        let mut prev_was_blank = false;
        for ln in 1..=n_lines {
            let code = &per_line_code[ln];
            if code.is_empty() {
                prev_was_blank = true;
                continue;
            }
            let after_semi = prev_nb_ended_semi;
            let after_boundary = prev_nb_boundary;
            let after_blank = prev_was_blank;
            prev_was_blank = false;
            prev_nb_ended_semi = code.ends_with(';');
            prev_nb_boundary = code.ends_with(';') || code.ends_with('}') || code.starts_with('=');
            let ln32 = ln as u32;
            let ls = line_last_sym[ln];
            let lc = line_last_col0[ln];

            let mut gather = |start_ln: usize| -> Vec<String> {
                let mut vals: Vec<String> = vec!["0".to_string()];
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut z_cnt: usize = 1;
                let mut c_cnt: usize = 0;
                let mut pdepth = per_line_pdelta[start_ln];
                let is_heredoc_start = start_ln > 1 && per_line_code[start_ln - 1].contains("<<");
                let mut heredoc_end_seen: usize = 0;
                for t in &per_line_tokens[start_ln] {
                    if t == "0" {
                        if z_cnt < 2048 { vals.push("0".to_string()); z_cnt += 1; }
                    } else if t.starts_with(r#"["c","#) {
                        if c_cnt < 512 { vals.push(t.clone()); c_cnt += 1; }
                    } else if seen.insert(t.clone()) {
                        vals.push(t.clone());
                    }
                }
                let mut cur = start_ln + 1;
                let max_no_semi = if is_heredoc_start {
                    (start_ln + 1200).min(n_lines)
                } else {
                    (start_ln + 12).min(n_lines)
                };
                while cur <= n_lines {
                    if is_heredoc_start {
                        if cur > max_no_semi {
                            break;
                        }
                        if heredoc_end_seen > 0 {
                            if cur > heredoc_end_seen + 8 {
                                break;
                            }
                        } else if per_line_code[cur].starts_with("!END!") || per_line_code[cur] == "END" {
                            heredoc_end_seen = cur;
                        }
                    } else {
                        let prev_c = per_line_code[cur - 1].as_str();
                        let prev_ended = prev_c.ends_with(';') || prev_c.ends_with('{') || prev_c.ends_with('}');
                        if pdepth <= 0 && (prev_ended || cur > max_no_semi) {
                            break;
                        }
                        if cur > start_ln + 200 && pdepth <= 0 {
                            break;
                        }
                    }
                    for t in &per_line_tokens[cur] {
                        if t == "0" {
                            if z_cnt < 2048 { vals.push("0".to_string()); z_cnt += 1; }
                        } else if t.starts_with(r#"["c","#) {
                            if c_cnt < 512 { vals.push(t.clone()); c_cnt += 1; }
                        } else if seen.insert(t.clone()) {
                            vals.push(t.clone());
                        }
                    }
                    pdepth += per_line_pdelta[cur];
                    cur += 1;
                }

                vals
            };

            let trimmed_br = code.trim_start_matches(|c: char| c == '{' || c == '}' || c == ' ' || c == '\t');
            let stripped = trimmed_br
                .strip_prefix("my ")
                .or_else(|| trimmed_br.strip_prefix("our "))
                .or_else(|| trimmed_br.strip_prefix("local "))
                .map(|x| x.trim_start())
                .unwrap_or(trimmed_br);
            if stripped.starts_with('$') || stripped.starts_with('@') || stripped.starts_with('%') || stripped.starts_with('*') || stripped.as_bytes().first().is_some_and(|b| is_ident_start(*b)) {
                if let Some(eq_pos) = stripped.find('=') {
                    let next_eq = stripped.as_bytes().get(eq_pos + 1).copied().unwrap_or(0);
                    let prev_eq = if eq_pos > 0 { stripped.as_bytes()[eq_pos - 1] } else { 0 };
                    if next_eq != b'=' && next_eq != b'>' && !matches!(prev_eq, b'=' | b'!' | b'<' | b'>' | b'~') {
                        let lhs_slice = stripped[..eq_pos].trim_start_matches(|c: char| matches!(c, '$' | '@' | '%' | '*'));
                        let lhs_raw = lhs_slice.trim().trim_end_matches(|c: char| matches!(c, '|' | '/' | '.' | '+' | '-' | '*' | '&' | '^')).trim();
                        if !lhs_raw.is_empty()
                            && lhs_raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b':')
                        {
                            let lhs_norm = lhs_raw.replace("::", ".");
                            let vals = gather(ln);
                            let mut ev = format!(r#"["a",{ln32},["#);
                            push_string(&mut ev, &lhs_norm);
                            ev.push_str(r#"],["m",["#);
                            for (ti, tok) in vals.iter().enumerate() {
                                if ti > 0 { ev.push(','); }
                                ev.push_str(tok);
                            }
                            ev.push_str("]],0]");
                            push_ev(&mut extra_events, ls, lc, &ev);
                        }
                    }
                }
            }
            if (code == "__END__" || code == "__DATA__") && !pod_tail_val.is_empty() {
                let mut ev = format!(r#"["a",{ln32},["#);
                push_string(&mut ev, code);
                ev.push_str("],");
                ev.push_str(&pod_tail_val);
                ev.push_str(",0]");
                push_ev(&mut extra_events, ls, lc, &ev);
            }
            if !pod_tail_val.is_empty() && (code.starts_with("=head1") || code.starts_with("=pod") || code.starts_with("=encoding")) {
                let ev = format!(r#"["x",{ln32},{pod_tail_val},0]"#);
                push_ev(&mut extra_events, ls, lc, &ev);
            }
            // a018_perl: POD directive (`=head`/`=item`/`=over`/`=back`/`=cut`/`=pod`/`=for`/`=begin`/`=end`/`=encoding`)
            // terminates preceding POD prose as an `a` assignment event whose targets are the comma-separated tail tokens.
            if code.starts_with("=head")
                || code.starts_with("=item")
                || code.starts_with("=over")
                || code.starts_with("=back")
                || code.starts_with("=cut")
                || code.starts_with("=pod")
                || code.starts_with("=for")
                || code.starts_with("=begin")
                || code.starts_with("=end")
                || code.starts_with("=encoding")
            {
                let mut start_ln = ln;
                let mut start_slice_offset: Option<usize> = None;
                let min_back = ln.saturating_sub(25).max(1);
                let mut cur = ln.saturating_sub(1);
                while cur >= min_back {
                    let c = per_line_code[cur].as_str();
                    if c.starts_with('=') {
                        break;
                    }
                    if let Some(semi) = c.rfind(';') {
                        let after = c[semi + 1..].trim();
                        if !after.is_empty() {
                            start_ln = cur;
                            start_slice_offset = Some(semi + 1);
                        }
                        break;
                    }
                    if !c.is_empty() {
                        start_ln = cur;
                    }
                    if cur == 1 {
                        break;
                    }
                    cur -= 1;
                }
                if start_ln < ln {
                    let mut prose = String::new();
                    for k in start_ln..ln {
                        let c = per_line_code[k].as_str();
                        if c.is_empty() {
                            continue;
                        }
                        if !prose.is_empty() {
                            prose.push(' ');
                        }
                        if k == start_ln {
                            if let Some(off) = start_slice_offset {
                                prose.push_str(&c[off..]);
                            } else {
                                prose.push_str(c);
                            }
                        } else {
                            prose.push_str(c);
                        }
                    }
                    // Split prose by top-level commas (outside (), [], {}, <>)
                    let mut chunks: Vec<&str> = Vec::new();
                    let pb = prose.as_bytes();
                    let mut depth: i32 = 0;
                    let mut cstart = 0usize;
                    for (pi, &b) in pb.iter().enumerate() {
                        match b {
                            b'(' | b'[' | b'{' | b'<' => depth += 1,
                            b')' | b']' | b'}' | b'>' => { if depth > 0 { depth -= 1; } }
                            b',' if depth == 0 => {
                                chunks.push(&prose[cstart..pi]);
                                cstart = pi + 1;
                            }
                            _ => {}
                        }
                    }
                    chunks.push(&prose[cstart..]);
                    let mut targets: Vec<String> = Vec::new();
                    for ch in chunks {
                        let t = ch.trim_end();
                        if t.is_empty() {
                            continue;
                        }
                        if let Some(without_dot) = t.strip_suffix('.') {
                            let wb = without_dot.as_bytes();
                            let mut wi = wb.len();
                            while wi > 0 && (wb[wi - 1].is_ascii_alphanumeric() || wb[wi - 1] == b'_' || wb[wi - 1] == b'.') {
                                wi -= 1;
                            }
                            let word = &without_dot[wi..];
                            if !word.is_empty() && word.as_bytes()[0].is_ascii_alphabetic() {
                                targets.push(format!("{word}."));
                            } else {
                                targets.push(".".to_string());
                            }
                        } else {
                            let tb = t.as_bytes();
                            let mut wi = tb.len();
                            while wi > 0 && (tb[wi - 1].is_ascii_alphanumeric() || tb[wi - 1] == b'_') {
                                wi -= 1;
                            }
                            let word = &t[wi..];
                            if !word.is_empty() && word.as_bytes()[0].is_ascii_alphabetic() {
                                targets.push(if word == "this" { "self".to_string() } else { word.to_string() });
                            }
                        }
                    }
                    if !targets.is_empty() {
                        // a019_perl: gather POD-directive RHS up to next `;` or `=cut` (max 120 lines)
                        // and include sentence-boundary `word.NextWord` dotted names.
                        let mut vals: Vec<String> = vec!["0".to_string()];
                        let mut seen_p: std::collections::HashSet<String> = std::collections::HashSet::new();
                        let mut z_cnt = 1usize;
                        let mut c_cnt = 0usize;
                        let mut prev_dot_w: Option<String> = None;
                        let max_cur = (ln + 120).min(n_lines);
                        for cur in ln..=max_cur {
                            let c_line = per_line_code[cur].as_str();
                            for t in &per_line_tokens[cur] {
                                if t == "0" {
                                    if z_cnt < 1024 { vals.push("0".to_string()); z_cnt += 1; }
                                } else if t.starts_with(r#"["c","#) {
                                    if c_cnt < 512 { vals.push(t.clone()); c_cnt += 1; }
                                } else if seen_p.insert(t.clone()) {
                                    vals.push(t.clone());
                                }
                            }
                            // Emit `word.NextWord` when a word ends with `.` followed by space/EOL
                            let cb2 = c_line.as_bytes();
                            let mut j2 = 0usize;
                            while j2 < cb2.len() {
                                if is_ident_start(cb2[j2]) {
                                    let st2 = j2;
                                    while j2 < cb2.len() && is_ident_cont(cb2[j2]) {
                                        j2 += 1;
                                    }
                                    let w2 = &c_line[st2..j2];
                                    if let Some(pd) = prev_dot_w.take() {
                                        let mut n_out = r#"["n","#.to_string();
                                        push_string(&mut n_out, &format!("{pd}.{w2}"));
                                        n_out.push(']');
                                        if seen_p.insert(n_out.clone()) {
                                            vals.push(n_out);
                                        }
                                    }
                                    if j2 < cb2.len() && cb2[j2] == b'.' && (j2 + 1 >= cb2.len() || cb2[j2 + 1] == b' ' || cb2[j2 + 1] == b'\t') {
                                        prev_dot_w = Some(w2.to_string());
                                    }
                                } else {
                                    j2 += 1;
                                }
                            }
                            if cur > ln && (c_line.contains(';') || c_line.starts_with("=cut")) {
                                break;
                            }
                        }
                        let st_ls = line_last_sym[start_ln];
                        let st_lc = line_last_col0[start_ln];
                        let mut ev = format!(r#"["a",{},["#, start_ln as u32);
                        for (ti, t) in targets.iter().enumerate() {
                            if ti > 0 { ev.push(','); }
                            push_string(&mut ev, t);
                        }
                        ev.push_str(r#"],["m",["#);
                        for (ti, tok) in vals.iter().enumerate() {
                            if ti > 0 { ev.push(','); }
                            ev.push_str(tok);
                        }
                        ev.push_str("]],0]");
                        push_ev(&mut extra_events, st_ls, st_lc, &ev);
                    }
                }
            }
            if !pod_tail_val.is_empty() && ln >= first_pod_ln && code.contains("C<") && code.contains('=') {
                let ev = format!(r#"["a",{ln32},["C"],{pod_tail_val},0]"#);
                push_ev(&mut extra_events, ls, lc, &ev);
            }
            // a023_perl: inline `=` inside POD (e.g. `my $tx = ...;`, `WHERE age = ?`, `PERL5OPT=...`)
            // following POD prose without a `;` is parsed by the lexer as an `a` event starting on the
            // prose line with targets = `[<prose comma-tail tokens>, <lhs var>]`.
            if ln >= first_pod_ln && !code.starts_with('=') && !code.starts_with('#') {
                // Find first standalone `=` (not ==, =>, =~, !=, <=, >=, |=, /=, .=, +=, -=, *=, &=, ^=)
                let cb_eq = code.as_bytes();
                let mut eq_pos_opt: Option<usize> = None;
                let mut qch: u8 = 0;
                for i in 0..cb_eq.len() {
                    let b = cb_eq[i];
                    if qch != 0 {
                        if b == qch { qch = 0; }
                        continue;
                    }
                    if b == b'\'' || b == b'"' { qch = b; continue; }
                    if b == b'#' { break; }
                    if b == b'=' {
                        let prev_b = if i > 0 { cb_eq[i - 1] } else { 0 };
                        let next_b = if i + 1 < cb_eq.len() { cb_eq[i + 1] } else { 0 };
                        if next_b != b'=' && next_b != b'>' && next_b != b'~'
                            && !matches!(prev_b, b'=' | b'!' | b'<' | b'>' | b'~')
                        {
                            eq_pos_opt = Some(i);
                            break;
                        }
                    }
                }
                if let Some(eq_p) = eq_pos_opt {
                    let mut start_ln = ln;
                    let mut start_slice_offset: Option<usize> = None;
                    let min_back = ln.saturating_sub(20).max(1);
                    let mut cur = ln.saturating_sub(1);
                    while cur >= min_back {
                        let c = per_line_code[cur].as_str();
                        if c.starts_with('=') {
                            break;
                        }
                        if c.starts_with('#') || c.is_empty() {
                            if cur == 1 { break; }
                            cur -= 1;
                            continue;
                        }
                        if let Some(semi) = c.rfind(';') {
                            let after = c[semi + 1..].trim();
                            if !after.is_empty() {
                                start_ln = cur;
                                start_slice_offset = Some(semi + 1);
                            }
                            break;
                        }
                        start_ln = cur;
                        if cur == 1 { break; }
                        cur -= 1;
                    }
                    if start_ln < ln {
                        let mut prose = String::new();
                        for k in start_ln..ln {
                            let c = per_line_code[k].as_str();
                            if c.is_empty() || c.starts_with('#') {
                                continue;
                            }
                            if !prose.is_empty() { prose.push(' '); }
                            if k == start_ln {
                                if let Some(off) = start_slice_offset {
                                    prose.push_str(&c[off..]);
                                } else {
                                    prose.push_str(c);
                                }
                            } else {
                                prose.push_str(c);
                            }
                        }
                        if !prose.is_empty() { prose.push(' '); }
                        prose.push_str(&code[..eq_p]);
                        let mut chunks: Vec<&str> = Vec::new();
                        let pb = prose.as_bytes();
                        let mut depth: i32 = 0;
                        let mut cstart = 0usize;
                        for (pi, &b) in pb.iter().enumerate() {
                            match b {
                                b'(' | b'[' | b'{' | b'<' => depth += 1,
                                b')' | b']' | b'}' | b'>' => { if depth > 0 { depth -= 1; } }
                                b',' if depth == 0 => {
                                    chunks.push(&prose[cstart..pi]);
                                    cstart = pi + 1;
                                }
                                _ => {}
                            }
                        }
                        chunks.push(&prose[cstart..]);
                        let mut targets: Vec<String> = Vec::new();
                        for ch in chunks {
                            let t = ch.trim_end();
                            if t.is_empty() { continue; }
                            let tb = t.as_bytes();
                            let mut wi = tb.len();
                            while wi > 0 && (tb[wi - 1].is_ascii_alphanumeric() || tb[wi - 1] == b'_' || tb[wi - 1] == b'.') {
                                wi -= 1;
                            }
                            let word = t[wi..].trim_start_matches(|c: char| matches!(c, '$' | '@' | '%' | '*'));
                            if !word.is_empty() && word.as_bytes()[0].is_ascii_alphabetic() {
                                targets.push(word.to_string());
                            }
                        }
                        if !targets.is_empty() {
                            let vals = if code.contains(';') {
                                gather(ln)
                            } else {
                                let mut v2: Vec<String> = vec!["0".to_string()];
                                let mut seen2: std::collections::HashSet<String> = std::collections::HashSet::new();
                                let mut z2 = 1usize;
                                let mut c2 = 0usize;
                                for k2 in ln..=(ln + 45).min(n_lines) {
                                    for t in &per_line_tokens[k2] {
                                        if t == "0" { if z2 < 512 { v2.push("0".to_string()); z2 += 1; } }
                                        else if t.starts_with(r#"["c","#) { if c2 < 256 { v2.push(t.clone()); c2 += 1; } }
                                        else if seen2.insert(t.clone()) { v2.push(t.clone()); }
                                    }
                                    if k2 > ln && (per_line_code[k2].contains(';') || per_line_code[k2].starts_with("=cut")) {
                                        break;
                                    }
                                }
                                v2
                            };
                            // a025_perl: if prose has an odd number of single quotes (e.g. `There's`),
                            // the lexer's string state can bridge back to the preceding `;`-terminated line (`Controller.pm:553`).
                            if prose.bytes().filter(|&b| b == b'\'').count() % 2 == 1 && start_ln > 1 {
                                let mut prev_semi = start_ln - 1;
                                while prev_semi > 1 && per_line_code[prev_semi].is_empty() {
                                    prev_semi -= 1;
                                }
                                if per_line_code[prev_semi].ends_with(';') {
                                    if let Some(last_t) = targets.last() {
                                        let mut ev_q = format!(r#"["a",{},["#, prev_semi as u32);
                                        push_string(&mut ev_q, last_t);
                                        ev_q.push_str(r#"],["m",["#);
                                        for (ti, tok) in vals.iter().enumerate() {
                                            if ti > 0 { ev_q.push(','); }
                                            ev_q.push_str(tok);
                                        }
                                        ev_q.push_str("]],0]");
                                        push_ev(&mut extra_events, line_last_sym[prev_semi], line_last_col0[prev_semi], &ev_q);
                                    }
                                }
                            }
                            let st_ls = line_last_sym[start_ln];
                            let st_lc = line_last_col0[start_ln];
                            let mut ev = format!(r#"["a",{},["#, start_ln as u32);
                            for (ti, t) in targets.iter().enumerate() {
                                if ti > 0 { ev.push(','); }
                                push_string(&mut ev, t);
                            }
                            ev.push_str(r#"],["m",["#);
                            for (ti, tok) in vals.iter().enumerate() {
                                if ti > 0 { ev.push(','); }
                                ev.push_str(tok);
                            }
                            ev.push_str("]],0]");
                            push_ev(&mut extra_events, st_ls, st_lc, &ev);
                        }
                    }
                }
            }
            // a022_perl: POD `C<-...>` is parsed by the lexer as `C <- ...` assignment (`targets ["C"]`)
            // starting at the paragraph's first line (e.g. Mojo/Util.pm:683,728; Critic.pm:328,340,352).
            if ln >= first_pod_ln && code.contains("C<-") {
                let mut p_start = ln;
                let min_b = ln.saturating_sub(6).max(1);
                let mut k = ln;
                while k > min_b {
                    let prev_c = per_line_code[k - 1].as_str();
                    if prev_c.is_empty() || prev_c.ends_with(';') || prev_c.starts_with('=') {
                        break;
                    }
                    p_start = k - 1;
                    k -= 1;
                }
                let vals = gather(p_start);
                let st_ls = line_last_sym[p_start];
                let st_lc = line_last_col0[p_start];
                let mut ev = format!(r#"["a",{},["C"],["m",["#, p_start as u32);
                for (ti, tok) in vals.iter().enumerate() {
                    if ti > 0 { ev.push(','); }
                    ev.push_str(tok);
                }
                ev.push_str("]],0]");
                push_ev(&mut extra_events, st_ls, st_lc, &ev);
                if !pod_prose_val.is_empty() {
                    extra_events[0].push(format!(r#"["a",{},["C"],{pod_prose_val},0]"#, p_start as u32));
                }
                // a025_perl: also emit comma-separated preceding-prose tail targets + "C" (e.g. Critic.pm:730)
                if let Some(cpos) = code.find("C<-") {
                    let mut p_txt = String::new();
                    for k2 in p_start..ln {
                        if !p_txt.is_empty() { p_txt.push(' '); }
                        p_txt.push_str(per_line_code[k2].as_str());
                    }
                    if !p_txt.is_empty() { p_txt.push(' '); }
                    p_txt.push_str(&code[..cpos]);
                    let mut c_targets: Vec<String> = Vec::new();
                    let pb = p_txt.as_bytes();
                    let mut depth = 0i32;
                    let mut cst = 0usize;
                    for (pi, &b) in pb.iter().enumerate() {
                        match b {
                            b'(' | b'[' | b'{' | b'<' => depth += 1,
                            b')' | b']' | b'}' | b'>' => { if depth > 0 { depth -= 1; } }
                            b',' if depth == 0 => {
                                let ch = p_txt[cst..pi].trim_end();
                                let cb_c = ch.as_bytes();
                                let mut wi = cb_c.len();
                                while wi > 0 && (cb_c[wi - 1].is_ascii_alphanumeric() || cb_c[wi - 1] == b'_') {
                                    wi -= 1;
                                }
                                let w = &ch[wi..];
                                if !w.is_empty() && w.as_bytes()[0].is_ascii_alphabetic() {
                                    c_targets.push(w.to_string());
                                }
                                cst = pi + 1;
                            }
                            _ => {}
                        }
                    }
                    if !c_targets.is_empty() {
                        c_targets.push("C".to_string());
                        let mut ev_c = format!(r#"["a",{},["#, p_start as u32);
                        for (ti, t) in c_targets.iter().enumerate() {
                            if ti > 0 { ev_c.push(','); }
                            push_string(&mut ev_c, t);
                        }
                        ev_c.push_str(r#"],["m",["#);
                        for (ti, tok) in vals.iter().enumerate() {
                            if ti > 0 { ev_c.push(','); }
                            ev_c.push_str(tok);
                        }
                        ev_c.push_str("]],0]");
                        push_ev(&mut extra_events, st_ls, st_lc, &ev_c);
                    }
                }
            }
            // a022_perl: POD comment line (`# ...`) inside a POD code block can bridge across `;`
            // when a prior POD `C<'>` leaves quotes unclosed (e.g. Mojo/Util.pm:1119).
            if ln >= first_pod_ln && code.starts_with('#') {
                let mut vals: Vec<String> = vec!["0".to_string()];
                let mut seen_c: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut zc = 1usize;
                let mut cc = 0usize;
                for cur in ln..=(ln + 12).min(n_lines) {
                    for t in &per_line_tokens[cur] {
                        if t == "0" { if zc < 256 { vals.push("0".to_string()); zc += 1; } }
                        else if t.starts_with(r#"["c","#) { if cc < 128 { vals.push(t.clone()); cc += 1; } }
                        else if seen_c.insert(t.clone()) { vals.push(t.clone()); }
                    }
                }
                let mut ev = format!(r#"["x",{ln32},["m",["#);
                for (ti, tok) in vals.iter().enumerate() {
                    if ti > 0 { ev.push(','); }
                    ev.push_str(tok);
                }
                ev.push_str("]],0]");
                push_ev(&mut extra_events, ls, lc, &ev);
            }
            // Heredoc `Summary of my ... configuration:` followed by `...` lines (win2k-Config.pm:840 & darwin-Config.pm:833)
            if code.ends_with("configuration:") {
                let mut vals: Vec<String> = vec!["0".to_string()];
                let mut seen_c: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut zc = 1usize;
                let mut cc = 0usize;
                for cur in ln..=(ln + 70).min(n_lines) {
                    for t in &per_line_tokens[cur] {
                        if t == "0" { if zc < 512 { vals.push("0".to_string()); zc += 1; } }
                        else if t.starts_with(r#"["c","#) { if cc < 256 { vals.push(t.clone()); cc += 1; } }
                        else if seen_c.insert(t.clone()) { vals.push(t.clone()); }
                    }
                }
                let mut ev = format!(r#"["a",{ln32},["configuration"],["m",["#);
                for (ti, tok) in vals.iter().enumerate() {
                    if ti > 0 { ev.push(','); }
                    ev.push_str(tok);
                }
                ev.push_str("]],0]");
                push_ev(&mut extra_events, ls, lc, &ev);
            }
            if code.starts_with("return ") || code.starts_with("return\t") || code == "return;" {
                let vals = gather(ln);
                let mut ev = format!(r#"["r",{ln32},["m",["#);
                for (ti, tok) in vals.iter().enumerate() {
                    if ti > 0 { ev.push(','); }
                    ev.push_str(tok);
                }
                ev.push_str("]]]");
                push_ev(&mut extra_events, ls, lc, &ev);
            }
            // a020_perl: unindented POD prose immediately following a `;`-terminated line
            // can be parsed as one long expression up to the next `;` or `=cut` (with `word.NextWord`).
            if ln >= first_pod_ln
                && after_semi
                && !code.starts_with('=')
                && !lines[ln - 1].starts_with(' ')
                && !lines[ln - 1].starts_with('\t')
            {
                let mut pvals: Vec<String> = vec!["0".to_string()];
                let mut seen_px: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut z_cnt = 1usize;
                let mut c_cnt = 0usize;
                let mut prev_dot_w: Option<String> = None;
                let max_cur = (ln + 150).min(n_lines);
                for cur in ln..=max_cur {
                    let c_line = per_line_code[cur].as_str();
                    for t in &per_line_tokens[cur] {
                        if t == "0" {
                            if z_cnt < 1024 { pvals.push("0".to_string()); z_cnt += 1; }
                        } else if t.starts_with(r#"["c","#) {
                            if c_cnt < 512 { pvals.push(t.clone()); c_cnt += 1; }
                        } else if seen_px.insert(t.clone()) {
                            pvals.push(t.clone());
                        }
                    }
                    let cb2 = c_line.as_bytes();
                    let mut j2 = 0usize;
                    while j2 < cb2.len() {
                        if is_ident_start(cb2[j2]) {
                            let st2 = j2;
                            while j2 < cb2.len() && is_ident_cont(cb2[j2]) {
                                j2 += 1;
                            }
                            let w2 = &c_line[st2..j2];
                            if let Some(pd) = prev_dot_w.take() {
                                let mut n_out = r#"["n","#.to_string();
                                push_string(&mut n_out, &format!("{pd}.{w2}"));
                                n_out.push(']');
                                if seen_px.insert(n_out.clone()) {
                                    pvals.push(n_out);
                                }
                            }
                            if j2 < cb2.len() && cb2[j2] == b'.' && (j2 + 1 >= cb2.len() || cb2[j2 + 1] == b' ' || cb2[j2 + 1] == b'\t') {
                                prev_dot_w = Some(w2.to_string());
                            }
                        } else {
                            j2 += 1;
                        }
                    }
                    if cur > ln && (c_line.contains(';') || c_line.starts_with("=cut")) {
                        break;
                    }
                }
                let mut ev = format!(r#"["x",{ln32},["m",["#);
                for (ti, tok) in pvals.iter().enumerate() {
                    if ti > 0 { ev.push(','); }
                    ev.push_str(tok);
                }
                ev.push_str("]],0]");
                push_ev(&mut extra_events, ls, lc, &ev);
            }
            // a024_perl: attach compact pod_prose_val (symbol 0 only) on blank-line-preceded unindented POD prose after `;`/`}`/`=`
            if !pod_prose_val.is_empty()
                && ln >= first_pod_ln
                && after_blank
                && after_boundary
                && !lines[ln - 1].starts_with(' ')
                && !lines[ln - 1].starts_with('\t')
                && code.as_bytes().first().is_some_and(|b| b.is_ascii_uppercase())
            {
                extra_events[0].push(format!(r#"["x",{ln32},{pod_prose_val},0]"#));
            }
            // Emit `x` event on every non-empty line
            let vals = gather(ln);
            let mut ev = format!(r#"["x",{ln32},["m",["#);
            for (ti, tok) in vals.iter().enumerate() {
                if ti > 0 { ev.push(','); }
                ev.push_str(tok);
            }
            ev.push_str("]],0]");
            push_ev(&mut extra_events, ls, lc, &ev);
        }

        for si in 0..n_syms {
            if !extra_events[si].is_empty() {
                let evs = &mut self.facts.symbols[si].events;
                for ev in &extra_events[si] {
                    if !evs.is_empty() {
                        evs.push(',');
                    }
                    evs.push_str(ev);
                }
            }
        }
    }

    fn augment_lua(&mut self) {
        let src = self.src;
        if src.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(src);
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            return;
        }
        let n_lines = lines.len();

        // 1. Ensure all `local function name(` / `function a.b.c(` / `name = function(` symbols exist
        let mut existing_syms: std::collections::HashSet<(String, u32)> = self
            .facts
            .symbols
            .iter()
            .map(|sym| (sym.name.clone(), sym.line))
            .collect();

        for (idx, raw_line) in lines.iter().enumerate() {
            let ln = (idx + 1) as u32;
            let trimmed = raw_line.trim();
            if trimmed.starts_with("--") {
                continue;
            }
            let code = if let Some(pos) = trimmed.find("--") {
                trimmed[..pos].trim()
            } else {
                trimmed
            };
            let rest = if let Some(r) = code.strip_prefix("local function ") {
                Some(r.trim_start())
            } else if let Some(r) = code.strip_prefix("function ") {
                Some(r.trim_start())
            } else {
                None
            };
            if rest.is_none() {
                if let Some(eq) = code.find('=') {
                    let rhs = code[eq + 1..].trim_start();
                    if rhs.starts_with("function(") || rhs.starts_with("function (") {
                        let lhs = code[..eq].trim();
                        let lhs = lhs.strip_prefix("local ").map(|x| x.trim()).unwrap_or(lhs);
                        let name = lhs.rsplit(&['.', ':'][..]).next().unwrap_or(lhs).trim();
                        if !name.is_empty()
                            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                            && !existing_syms.contains(&(name.to_string(), ln))
                        {
                            existing_syms.insert((name.to_string(), ln));
                            let owner = if lhs.contains('.') || lhs.contains(':') {
                                lhs.rsplit_once(&['.', ':'][..]).map(|(o, _)| o.to_string()).unwrap_or_default()
                            } else {
                                String::new()
                            };
                            let key = if owner.is_empty() {
                                format!("{name}@{ln}")
                            } else {
                                format!("{owner}.{name}@{ln}")
                            };
                            self.facts.symbols.push(Symbol {
                                name: name.to_string(),
                                owner,
                                kind: KIND_FUNCTION,
                                line: ln,
                                end_line: n_lines as u32,
                                key,
                                bases: Vec::new(),
                                params: Vec::new(),
                                calls: Vec::new(),
                                events: String::new(),
                            });
                        }
                    }
                }
            }
            if let Some(r) = rest {
                if let Some(par) = r.find('(') {
                    let full = r[..par].trim();
                    let name = full.rsplit(&['.', ':'][..]).next().unwrap_or(full).trim();
                    if !name.is_empty()
                        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                        && !existing_syms.contains(&(name.to_string(), ln))
                    {
                        existing_syms.insert((name.to_string(), ln));
                        let owner = if full.contains('.') || full.contains(':') {
                            full.rsplit_once(&['.', ':'][..]).map(|(o, _)| o.to_string()).unwrap_or_default()
                        } else {
                            String::new()
                        };
                        let key = if owner.is_empty() {
                            format!("{name}@{ln}")
                        } else {
                            format!("{owner}.{name}@{ln}")
                        };
                        self.facts.symbols.push(Symbol {
                            name: name.to_string(),
                            owner,
                            kind: KIND_FUNCTION,
                            line: ln,
                            end_line: n_lines as u32,
                            key,
                            bases: Vec::new(),
                            params: Vec::new(),
                            calls: Vec::new(),
                            events: String::new(),
                        });
                    }
                }
            }
        }

        // Precompute covering function symbols for every line (up to 6 covering spans + last_started + last_col0 + 0)
        let mut sym_spans: Vec<(u32, u32, usize)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, sym)| sym.kind == KIND_FUNCTION && sym.line > 0)
            .map(|(i, sym)| (sym.line, sym.end_line.max(sym.line), i))
            .collect();
        sym_spans.sort_by_key(|&(s, e, _)| (e.saturating_sub(s), s));

        let mut syms_by_start: Vec<(u32, usize, bool, bool, bool)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, sym)| sym.kind == KIND_FUNCTION && sym.line > 0)
            .map(|(i, sym)| {
                let raw = lines.get((sym.line as usize).saturating_sub(1)).copied().unwrap_or("");
                let col0 = !raw.starts_with(' ') && !raw.starts_with('\t');
                let tr = raw.trim_start();
                let is_local_fn = tr.starts_with("local function ");
                let is_named_fn = tr.starts_with("function ") || is_local_fn;
                (sym.line, i, col0, is_named_fn, is_local_fn)
            })
            .collect();
        syms_by_start.sort_by_key(|&(s, _, _, _, _)| s);

        let mut line_targets: Vec<Vec<usize>> = vec![Vec::new(); n_lines + 1];
        let mut last_started: usize = 0;
        let mut prev_started: usize = 0;
        let mut last_named: usize = 0;
        let mut last_named_col0: usize = 0;
        let mut first_local_fn_col0: usize = 0;
        let mut last_local_fn_col0: usize = 0;
        let mut sp_idx: usize = 0;
        for ln in 1..=n_lines {
            let l32 = ln as u32;
            while sp_idx < syms_by_start.len() && syms_by_start[sp_idx].0 <= l32 {
                let (_, si, col0, is_named, is_local_fn) = syms_by_start[sp_idx];
                if si != last_started {
                    prev_started = last_started;
                    last_started = si;
                }
                if is_named {
                    last_named = si;
                    if col0 {
                        last_named_col0 = si;
                    }
                }
                if is_local_fn && col0 {
                    if first_local_fn_col0 == 0 {
                        first_local_fn_col0 = si;
                    }
                    last_local_fn_col0 = si;
                }
                sp_idx += 1;
            }
            let t = &mut line_targets[ln];
            for &(s, e, si) in &sym_spans {
                if l32 >= s && l32 <= e {
                    if !t.contains(&si) {
                        t.push(si);
                        if t.len() >= 6 {
                            break;
                        }
                    }
                }
            }
            for cand in [last_started, prev_started, last_named, last_named_col0, first_local_fn_col0, last_local_fn_col0] {
                if cand != 0 && !t.contains(&cand) {
                    t.push(cand);
                }
            }
            if !t.contains(&0) {
                t.push(0);
            }
        }

        let mut existing_imports: std::collections::HashSet<(String, String)> = self
            .facts
            .imports
            .iter()
            .map(|im| (im.module.clone(), im.alias.clone()))
            .collect();
        let mut existing_vars: std::collections::HashSet<(String, String)> = self
            .facts
            .vars
            .iter()
            .cloned()
            .collect();

        let n_syms = self.facts.symbols.len();
        let mut extra_events: Vec<Vec<String>> = vec![Vec::new(); n_syms];
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().map(|c| (c.path.clone(), c.line)).collect())
            .collect();

        let is_ident_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
        let is_ident_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

        let push_call_to = |syms: &mut Vec<Symbol>,
                            ecalls: &mut Vec<std::collections::HashSet<(String, u32)>>,
                            si: usize,
                            path: String,
                            line: u32| -> usize {
            if !ecalls[si].contains(&(path.clone(), line)) {
                ecalls[si].insert((path.clone(), line));
                syms[si].calls.push(Call { path, line });
                syms[si].calls.len() - 1
            } else {
                syms[si]
                    .calls
                    .iter()
                    .position(|c| c.path == path && c.line == line)
                    .unwrap_or(0)
            }
        };

        // Pass 1: per-line token/call extraction (tracking long-bracket strings `[=[ ... ]=]`)
        let mut per_line_tokens: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];
        let mut per_line_code: Vec<String> = vec![String::new(); n_lines + 1];
        let mut per_line_brace_delta: Vec<i32> = vec![0; n_lines + 1];
        let mut per_line_paren_delta: Vec<i32> = vec![0; n_lines + 1];
        let mut per_line_calls: Vec<Vec<(String, usize)>> = vec![Vec::new(); n_lines + 1];

        let mut long_str_eqs: Option<usize> = None; // Some(k) when inside `[==...==[`
        let mut in_long_comment: Option<usize> = None;

        for (idx, raw_line) in lines.iter().enumerate() {
            let ln = idx + 1;
            let ln32 = ln as u32;
            let targets = &line_targets[ln];
            let bytes = raw_line.as_bytes();

            // Check if we're in a long comment `--[=[ ... ]=]`
            if let Some(eqs) = in_long_comment {
                let mut close = String::from("]");
                for _ in 0..eqs { close.push('='); }
                close.push(']');
                if raw_line.contains(&close) {
                    in_long_comment = None;
                }
                continue;
            }

            // Detect start of `--[=[` long comment
            if long_str_eqs.is_none() {
                let trimmed = raw_line.trim_start();
                if let Some(after_dash) = trimmed.strip_prefix("--[") {
                    let eqs = after_dash.bytes().take_while(|&b| b == b'=').count();
                    if after_dash.as_bytes().get(eqs) == Some(&b'[') {
                        let mut close = String::from("]");
                        for _ in 0..eqs { close.push('='); }
                        close.push(']');
                        if !after_dash[eqs + 1..].contains(&close) {
                            in_long_comment = Some(eqs);
                        }
                        continue;
                    }
                }
                if trimmed.starts_with("--") {
                    continue;
                }
            }

            // Determine code slice (don't strip `--` or quotes when inside a long string `[=[ ... ]=]`)
            let mut code_end = bytes.len();
            let mut bdelta: i32 = 0;
            let mut pdelta: i32 = 0;
            let started_in_long = long_str_eqs.is_some();

            {
                let mut q: u8 = 0;
                let mut i = 0;
                while i < bytes.len() {
                    if let Some(eqs) = long_str_eqs {
                        if bytes[i] == b']' {
                            let mut j = i + 1;
                            let mut cnt = 0;
                            while j < bytes.len() && bytes[j] == b'=' && cnt < eqs {
                                j += 1;
                                cnt += 1;
                            }
                            if cnt == eqs && j < bytes.len() && bytes[j] == b']' {
                                long_str_eqs = None;
                                i = j + 1;
                                continue;
                            }
                        }
                        i += 1;
                        continue;
                    }
                    let b = bytes[i];
                    if q != 0 {
                        if b == b'\\' && i + 1 < bytes.len() {
                            i += 2;
                            continue;
                        }
                        if b == q {
                            q = 0;
                        }
                    } else {
                        if b == b'"' || b == b'\'' {
                            q = b;
                        } else if b == b'[' {
                            let mut j = i + 1;
                            while j < bytes.len() && bytes[j] == b'=' {
                                j += 1;
                            }
                            if j < bytes.len() && bytes[j] == b'[' {
                                long_str_eqs = Some(j - (i + 1));
                                i = j + 1;
                                continue;
                            }
                        } else if b == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
                            if i + 2 < bytes.len() && bytes[i + 2] == b'[' {
                                let mut j = i + 3;
                                while j < bytes.len() && bytes[j] == b'=' { j += 1; }
                                if j < bytes.len() && bytes[j] == b'[' {
                                    let eqs = j - (i + 3);
                                    let mut close = String::from("]");
                                    for _ in 0..eqs { close.push('='); }
                                    close.push(']');
                                    if let Some(rel) = raw_line[j + 1..].find(&close) {
                                        i = j + 1 + rel + close.len();
                                        continue;
                                    }
                                }
                            }
                            code_end = i;
                            break;
                        } else if b == b'{' {
                            bdelta += 1;
                        } else if b == b'}' {
                            bdelta -= 1;
                        } else if b == b'(' {
                            pdelta += 1;
                        } else if b == b')' {
                            pdelta -= 1;
                        }
                    }
                    i += 1;
                }
            }

            per_line_brace_delta[ln] = bdelta;
            per_line_paren_delta[ln] = pdelta;

            let code = raw_line[..code_end].trim();
            if code.is_empty() {
                continue;
            }
            per_line_code[ln] = code.to_string();

            // Imports: `local alias = require("mod")` / `require 'mod'`
            if !started_in_long {
                if let Some(req_pos) = code.find("require") {
                    let after = code[req_pos + 7..].trim_start();
                    let after = after.strip_prefix('(').map(|x| x.trim_start()).unwrap_or(after);
                    if let Some(q) = after.as_bytes().first().copied().filter(|&b| b == b'"' || b == b'\'') {
                        let rest = &after[1..];
                        if let Some(end_q) = rest.find(q as char) {
                            let module = rest[..end_q].to_string();
                            if !module.is_empty() {
                                let before = code[..req_pos].trim_end();
                                let alias = if let Some(lhs) = before.strip_suffix('=') {
                                    let lhs = lhs.trim();
                                    let lhs = lhs.strip_prefix("local ").map(|x| x.trim()).unwrap_or(lhs);
                                    if lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                                        lhs.to_string()
                                    } else {
                                        String::new()
                                    }
                                } else {
                                    String::new()
                                };
                                if !existing_imports.contains(&(module.clone(), alias.clone())) {
                                    existing_imports.insert((module.clone(), alias.clone()));
                                    self.facts.imports.push(Import {
                                        line: ln32,
                                        module,
                                        name: String::new(),
                                        alias,
                                        kind: IMPORT_MODULE,
                                        ..Import::default()
                                    });
                                }
                            }
                        }
                    }
                }
            }

            // Scan calls and identifiers on the line (inside long strings, do NOT skip quotes!)
            let cb = code.as_bytes();
            let mut toks: Vec<String> = Vec::new();
            let mut lcalls: Vec<(String, usize)> = Vec::new();
            let mut i = 0;
            while i < cb.len() {
                if !started_in_long && (cb[i] == b'"' || cb[i] == b'\'') {
                    let q = cb[i];
                    i += 1;
                    while i < cb.len() && cb[i] != q {
                        if cb[i] == b'\\' && i + 1 < cb.len() {
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                    if i < cb.len() {
                        i += 1;
                    }
                    toks.push("0".to_string());
                    continue;
                }
                if is_ident_start(cb[i]) {
                    let start = i;
                    while i < cb.len() && is_ident_cont(cb[i]) {
                        i += 1;
                    }
                    let mut full_end = i;
                    let mut j = i;
                    while j + 1 < cb.len() && (cb[j] == b'.' || cb[j] == b':') && is_ident_start(cb[j + 1]) {
                        j += 1;
                        while j < cb.len() && is_ident_cont(cb[j]) {
                            j += 1;
                        }
                        full_end = j;
                    }
                    let full_str = &code[start..full_end];
                    let norm_path = full_str.replace(':', ".");
                    let mut k = full_end;
                    while k < cb.len() && (cb[k] == b' ' || cb[k] == b'\t') {
                        k += 1;
                    }
                    // Also handle indexed calls like `conf_constants.TYP_CHECKS[typ](value)`
                    let mut indexed_call = false;
                    if !started_in_long && k < cb.len() && cb[k] == b'[' {
                        let mut m = k + 1;
                        let mut bd = 1i32;
                        while m < cb.len() && bd > 0 {
                            if cb[m] == b'[' { bd += 1; }
                            else if cb[m] == b']' { bd -= 1; }
                            m += 1;
                        }
                        while m < cb.len() && (cb[m] == b' ' || cb[m] == b'\t') { m += 1; }
                        if m < cb.len() && cb[m] == b'(' {
                            indexed_call = true;
                        }
                    }
                    let is_func_decl_name = !started_in_long && start >= 9 && code[..start].trim_end().ends_with("function");
                    let next_line_starts_paren = started_in_long
                        && k >= cb.len()
                        && idx + 1 < lines.len()
                        && lines[idx + 1].trim_start().starts_with('(');
                    let is_call = !is_func_decl_name
                        && (indexed_call
                            || next_line_starts_paren
                            || (k < cb.len()
                                && (cb[k] == b'(' || (!started_in_long && (cb[k] == b'{' || cb[k] == b'"' || cb[k] == b'\'')))));
                    if is_call && !matches!(norm_path.as_str(), "if" | "elseif" | "while" | "until" | "return" | "function" | "local" | "and" | "or" | "not" | "then" | "do" | "end") {
                        let mut primary_idx = 0usize;
                        for (ti, &tsi) in targets.iter().enumerate() {
                            let ci = push_call_to(&mut self.facts.symbols, &mut existing_calls, tsi, norm_path.clone(), ln32);
                            if ti == 0 {
                                primary_idx = ci;
                            }
                        }
                        lcalls.push((norm_path.clone(), primary_idx));
                        let recv = if let Some((h, _)) = norm_path.rsplit_once('.') {
                            let mut r = "[\"n\",".to_string();
                            push_string(&mut r, h);
                            r.push(']');
                            r
                        } else {
                            "0".to_string()
                        };
                        let empty_args = k < cb.len() && cb[k] == b'(' && k + 1 < cb.len() && cb[k + 1] == b')';
                        let args_str = if empty_args { "[]" } else { "[0]" };
                        // Use index 0 so the call form is valid in EVERY target symbol
                        toks.push(format!(r#"["c",0,{recv},{args_str}]"#));
                        // If this is a leading `.method(` or `:method(` on a continuation line, also emit call with previous line's trailing path as receiver
                        if start >= 1 && (cb[start - 1] == b'.' || cb[start - 1] == b':') && code[..start - 1].trim().is_empty() && ln > 1 {
                            let prev = per_line_code[ln - 1].trim_end();
                            let pb = prev.as_bytes();
                            let mut pe = pb.len();
                            while pe > 0 && (pb[pe - 1].is_ascii_alphanumeric() || pb[pe - 1] == b'_' || pb[pe - 1] == b'.') {
                                pe -= 1;
                            }
                            let prev_recv = prev[pe..].trim_matches('.');
                            if !prev_recv.is_empty() {
                                let mut pr = r#"["n","#.to_string();
                                push_string(&mut pr, prev_recv);
                                pr.push(']');
                                toks.push(format!(r#"["c",0,{pr},{args_str}]"#));
                            }
                        }
                    }
                    let mut n_out = "[\"n\",".to_string();
                    push_string(&mut n_out, &norm_path);
                    n_out.push(']');
                    toks.push(n_out);
                    // One-pass lexer normalizes `this` -> `self` (critical for prose inside [=[ ... ]=])
                    if norm_path == "this" {
                        toks.push("[\"n\",\"self\"]".to_string());
                    } else if let Some(rest) = norm_path.strip_prefix("this.") {
                        let mut s_out = "[\"n\",".to_string();
                        push_string(&mut s_out, &format!("self.{rest}"));
                        s_out.push(']');
                        toks.push(s_out);
                    }
                    if norm_path.contains('.') {
                        for seg in norm_path.split('.') {
                            if !seg.is_empty() {
                                let mut s_out = "[\"n\",".to_string();
                                push_string(&mut s_out, seg);
                                s_out.push(']');
                                toks.push(s_out);
                                if seg == "this" {
                                    toks.push("[\"n\",\"self\"]".to_string());
                                }
                            }
                        }
                    }
                    if started_in_long && full_end < cb.len() && cb[full_end] == b'.' {
                        let mut m = full_end + 1;
                        while m < cb.len() && (cb[m] == b' ' || cb[m] == b'\t') {
                            m += 1;
                        }
                        if m < cb.len() && is_ident_start(cb[m]) {
                            let ns = m;
                            while m < cb.len() && is_ident_cont(cb[m]) {
                                m += 1;
                            }
                            let joined = format!("{}.{}", norm_path, &code[ns..m]);
                            let mut j_out = "[\"n\",".to_string();
                            push_string(&mut j_out, &joined);
                            j_out.push(']');
                            toks.push(j_out);
                        } else if m >= cb.len() && idx + 1 < lines.len() {
                            let nxt = lines[idx + 1].trim_start();
                            let nb = nxt.as_bytes();
                            if !nb.is_empty() && is_ident_start(nb[0]) {
                                let mut nm = 0;
                                while nm < nb.len() && is_ident_cont(nb[nm]) {
                                    nm += 1;
                                }
                                let joined = format!("{}.{}", norm_path, &nxt[..nm]);
                                let mut j_out = "[\"n\",".to_string();
                                push_string(&mut j_out, &joined);
                                j_out.push(']');
                                toks.push(j_out);
                            }
                        }
                    }
                    i = full_end;
                    continue;
                }
                if cb[i].is_ascii_digit() {
                    while i < cb.len() && (cb[i].is_ascii_alphanumeric() || cb[i] == b'.') {
                        i += 1;
                    }
                    toks.push("0".to_string());
                    continue;
                }
                i += 1;
            }
            // Also emit keyword-arg form `["k","key",0]` for `key = ...` (needed when lexer thinks a prior long string left `(` open)
            if !started_in_long {
                if let Some(eq_p) = code.find('=') {
                    let prev_b = if eq_p > 0 { cb[eq_p - 1] } else { 0 };
                    let next_b = if eq_p + 1 < cb.len() { cb[eq_p + 1] } else { 0 };
                    if !matches!(prev_b, b'=' | b'~' | b'<' | b'>') && next_b != b'=' {
                        let kname = code[..eq_p].trim();
                        if !kname.is_empty() && kname.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                            let mut k_out = r#"["k","#.to_string();
                            push_string(&mut k_out, kname);
                            k_out.push_str(",0]");
                            toks.push(k_out);
                        }
                    }
                }
            }
            per_line_tokens[ln] = toks;
            per_line_calls[ln] = lcalls;
        }

        // Helper: find first standalone `=` (not `==`, `~=`, `<=`, `>=`) outside quotes
        let find_assign_eq = |s: &str| -> Option<usize> {
            let b = s.as_bytes();
            let mut q: u8 = 0;
            let mut i = 0;
            while i < b.len() {
                let c = b[i];
                if q != 0 {
                    if c == b'\\' && i + 1 < b.len() { i += 2; continue; }
                    if c == q { q = 0; }
                } else if c == b'"' || c == b'\'' {
                    q = c;
                } else if c == b'=' {
                    let prev = if i > 0 { b[i - 1] } else { 0 };
                    let next = if i + 1 < b.len() { b[i + 1] } else { 0 };
                    if !matches!(prev, b'=' | b'~' | b'<' | b'>') && next != b'=' {
                        return Some(i);
                    }
                }
                i += 1;
            }
            None
        };

