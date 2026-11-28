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
    if lang == Lang::R {
        let st = walker.augment_r_host_a();
        walker.augment_r_host_b(st);
    }
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
        walker.augment_php_host();
    }
    if lang == Lang::C {
        walker.augment_c();
    }
    if lang == Lang::Swift {
        walker.augment_swift_v2();
    }
    if lang == Lang::Rust {
        walker.augment_rust();
    }
    if lang == Lang::Scala {
        walker.augment_scala_v2();
    }
    if lang == Lang::Lua {
        let (lt, sb) = walker.augment_lua_host_a();
        let st = walker.augment_lua_host_b(&lt);
        walker.augment_lua_host_c(&lt, &sb, st);
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
    if lang == Lang::Scala {
        walker.augment_scala();
    }
    if lang == Lang::Swift {
        walker.augment_swift();
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


    fn augment_r_host_a(&mut self) -> (Vec<String>, Vec<i32>, Vec<Vec<String>>) {
        let src_str = std::str::from_utf8(self.src).unwrap_or("");
        let lines: Vec<&str> = src_str.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() {
            return (Vec::new(), Vec::new(), Vec::new());
        }
        let orig_n_syms = self.facts.symbols.len();
        let n_syms = self.facts.symbols.len();
        let is_id_start = |b: u8| b.is_ascii_alphabetic() || b == b'_' || b == b'.';
        let is_id_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'.';

        let mut existing_imports: std::collections::HashSet<(String, String, String, u8)> = self
            .facts
            .imports
            .iter()
            .map(|imp| (imp.module.clone(), imp.name.clone(), imp.alias.clone(), imp.kind))
            .collect();
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

        for (idx, raw_line) in lines.iter().enumerate() {
            let ln = idx + 1;
            let ln32 = ln as u32;
            let bytes = raw_line.as_bytes();
            let mut stripped = String::with_capacity(raw_line.len());
            let mut i = 0usize;
            let mut q: u8 = 0;
            let mut pdelta = 0i32;
            while i < bytes.len() {
                if q != 0 {
                    let b = bytes[i];
                    let ch = raw_line[i..].chars().next().unwrap_or(' ');
                    stripped.push(ch);
                    if b == b'\\' && i + 1 < bytes.len() {
                        i += 1;
                        let ch2 = raw_line[i..].chars().next().unwrap_or(' ');
                        stripped.push(ch2);
                        i += ch2.len_utf8();
                        continue;
                    }
                    if b == q { q = 0; }
                    i += ch.len_utf8();
                    continue;
                }
                if bytes[i] == b'#' { break; }
                let b = bytes[i];
                if b == b'"' || b == b'\'' || b == b'`' { q = b; }
                else if matches!(b, b'(' | b'[' | b'{') { pdelta += 1; }
                else if matches!(b, b')' | b']' | b'}') { pdelta -= 1; }
                let ch = raw_line[i..].chars().next().unwrap_or(' ');
                stripped.push(ch);
                i += ch.len_utf8();
            }
            let code = stripped.trim();
            if code.is_empty() { continue; }
            per_line_code[ln] = code.to_string();
            per_line_pdelta[ln] = pdelta;

            // Active symbols
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

            // R imports: library(...), require(...), source(...)
            for kw in ["library(", "require(", "source("] {
                let mut pos = 0usize;
                while let Some(rel) = code[pos..].find(kw) {
                    let st = pos + rel + kw.len();
                    pos = st;
                    let arg = code[st..].split(|c: char| matches!(c, ',' | ')')).next().unwrap_or("").trim();
                    let arg = arg.split('=').last().unwrap_or(arg).trim();
                    let mod_nm = arg.trim_matches(|c: char| matches!(c, '"' | '\'' | ' '));
                    let mod_base = mod_nm.rsplit('/').next().unwrap_or(mod_nm);
                    if !mod_base.is_empty() && mod_base.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-') {
                        for knd in [0u8, IMPORT_STAR] {
                            if existing_imports.insert((mod_base.to_string(), String::new(), mod_base.to_string(), knd)) {
                                self.facts.imports.push(Import {
                                    module: mod_base.to_string(), name: String::new(), alias: mod_base.to_string(),
                                    kind: knd, line: ln32,
                                });
                            }
                        }
                    }
                }
            }

            // R multiset var bindings (`lhs <- UpperIdent(` or `lhs = UpperIdent(`)
            if let Some((ep, olen)) = code.find("<-").map(|p| (p, 2usize)).or_else(|| code.find('=').map(|p| (p, 1usize))) {
                let prev_b = if ep > 0 { code.as_bytes()[ep - 1] } else { 0 };
                let next_b = code.as_bytes().get(ep + olen).copied().unwrap_or(0);
                if olen == 2 || (next_b != b'=' && !matches!(prev_b, b'=' | b'!' | b'<' | b'>')) {
                    let var_nm = code[..ep].trim().split_whitespace().last().unwrap_or("");
                    let rhs_s = code[ep + olen..].trim_start();
                    let rhs_id: String = rhs_s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.' || *c == ':').collect();
                    let rhs_ty = rhs_id.rsplit("::").next().unwrap_or(&rhs_id);
                    if !var_nm.is_empty() && !rhs_ty.is_empty() && rhs_ty.as_bytes()[0].is_ascii_uppercase() {
                        self.facts.vars.push((var_nm.to_string(), rhs_ty.to_string()));
                    }
                }
            }

            // Tokenize R line + emit calls
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
                if is_id_start(cb[j]) && !(cb[j] == b'.' && (j + 1 >= cb.len() || cb[j + 1].is_ascii_digit())) {
                    let st = j;
                    j += 1;
                    while j < cb.len() {
                        if is_id_cont(cb[j]) { j += 1; }
                        else if cb[j] == b'$' && j + 1 < cb.len() && is_id_start(cb[j + 1]) { j += 1; }
                        else if cb[j] == b':' && j + 1 < cb.len() && cb[j + 1] == b':' && j + 2 < cb.len() && is_id_start(cb[j + 2]) { j += 2; }
                        else { break; }
                    }
                    let raw_id = &code[st..j];
                    let norm = raw_id.replace('$', ".").replace("::", ".");
                    let mut kf = j;
                    while kf < cb.len() && (cb[kf] == b' ' || cb[kf] == b'\t') { kf += 1; }
                    if kf < cb.len() && cb[kf] == b'=' && (kf + 1 >= cb.len() || cb[kf + 1] != b'=') {
                        let mut k_out = r#"["k","#.to_string();
                        push_string(&mut k_out, &norm);
                        k_out.push_str(",0]");
                        toks.push(k_out);
                    }
                    if kf < cb.len() && cb[kf] == b'(' {
                        if !matches!(norm.as_str(), "if"|"for"|"while"|"repeat"|"function"|"return"|"next"|"break") {
                            let mut cands: Vec<String> = vec![norm.clone()];
                            if let Some((_, last_seg)) = norm.rsplit_once('.') {
                                if !last_seg.is_empty() { cands.push(last_seg.to_string()); }
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
                            if !h.is_empty() {
                                let mut r = r#"["n","#.to_string();
                                push_string(&mut r, h);
                                r.push(']');
                                r
                            } else { "0".to_string() }
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
                    if let Some(nodot) = norm.strip_prefix('.') {
                        if !nodot.is_empty() && nodot.as_bytes()[0].is_ascii_alphabetic() {
                            let mut nd = r#"["n","#.to_string();
                            push_string(&mut nd, nodot);
                            nd.push(']');
                            toks.push(nd);
                        }
                    }
                    if let Some((_, after_first_dot)) = norm.split_once('.') {
                        if !after_first_dot.is_empty() && after_first_dot.contains('.') {
                            let mut na = r#"["n","#.to_string();
                            push_string(&mut na, after_first_dot);
                            na.push(']');
                            toks.push(na);
                        }
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
                    | "new" | "sizeof" | "if" | "elif" | "else" | "for" | "while" | "repeat"
                    | "function" | "next" | "break"
            )
        };
        (per_line_code, per_line_pdelta, per_line_tokens)
    }

    fn augment_r_host_b(&mut self, state: (Vec<String>, Vec<i32>, Vec<Vec<String>>)) {
        let (per_line_code, per_line_pdelta, mut per_line_tokens) = state;
        let src_str = std::str::from_utf8(self.src).unwrap_or("");
        let lines: Vec<&str> = src_str.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() || per_line_code.len() <= n_lines { return; }
        let n_syms = self.facts.symbols.len();
        let orig_n_syms = n_syms;
        let is_id_start = |b: u8| b.is_ascii_alphabetic() || b == b'_' || b == b'.';
        let is_id_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'.';
        let mut existing_imports: std::collections::HashSet<(String, String, String, u8)> = self
            .facts.imports.iter().map(|imp| (imp.module.clone(), imp.name.clone(), imp.alias.clone(), imp.kind)).collect();
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts.symbols.iter().map(|sym| sym.calls.iter().map(|c| (c.path.clone(), c.line)).collect()).collect();
        let mut sym_spans: Vec<(u32, u32, usize)> = self
            .facts.symbols.iter().enumerate().skip(1).map(|(i, s)| (s.line, s.end_line.max(s.line), i)).collect();
        sym_spans.sort_by_key(|&(st, _, _)| st);
        let mut existing_vars: std::collections::HashSet<(String, String)> =
            self.facts.vars.iter().cloned().collect();
        let mut in_ml_dq = false;
        let mut ml_dq_lines: Vec<bool> = vec![false; n_lines + 1];
        for ln in 1..=n_lines {
            let mut dq_cnt = 0usize;
            let raw_b = lines[ln - 1].as_bytes();
            let mut bi = 0usize;
            while bi < raw_b.len() {
                if raw_b[bi] == b'\\' && bi + 1 < raw_b.len() { bi += 2; continue; }
                if !in_ml_dq && raw_b[bi] == b'#' { break; }
                if raw_b[bi] == b'"' { dq_cnt += 1; }
                bi += 1;
            }
            if in_ml_dq || (dq_cnt % 2 == 1) {
                ml_dq_lines[ln] = true;
            }
            if dq_cnt % 2 == 1 {
                in_ml_dq = !in_ml_dq;
            }
            let ln32 = ln as u32;
            let raw_t = lines[ln - 1].trim();
            if ln > 1 && raw_t.starts_with('(') {
                let prev_t = lines[ln - 2].trim_end();
                let pb = prev_t.as_bytes();
                let mut pi = pb.len();
                while pi > 0 && (pb[pi - 1].is_ascii_alphanumeric() || pb[pi - 1] == b'_' || pb[pi - 1] == b'.') {
                    pi -= 1;
                }
                let w = &prev_t[pi..];
                if !w.is_empty() && w.as_bytes()[0].is_ascii_alphabetic() {
                    if existing_calls[0].insert((w.to_string(), ln32)) {
                        self.facts.symbols[0].calls.push(Call { path: w.to_string(), line: ln32 });
                    }
                }
            }
            if ln < n_lines && !per_line_code[ln].trim().is_empty() {
                let c_cur = per_line_code[ln].trim_end();
                let mut nxt_i = ln + 1;
                while nxt_i < n_lines && per_line_code[nxt_i].trim().is_empty() { nxt_i += 1; }
                let c_nxt = per_line_code[nxt_i].trim_start();
                if c_nxt.starts_with('.') {
                    let c_cur = c_cur.trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
                    let cb = c_cur.as_bytes();
                    let mut ci = cb.len();
                    while ci > 0 && (cb[ci - 1].is_ascii_alphanumeric() || cb[ci - 1] == b'_') { ci -= 1; }
                    let w1 = &c_cur[ci..];
                    let w2: String = c_nxt.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
                    if !w1.is_empty() && w1.as_bytes()[0].is_ascii_alphabetic() && w2.len() > 1 {
                        let mut nj = r#"["n","#.to_string();
                        push_string(&mut nj, &format!("{w1}{w2}"));
                        nj.push(']');
                        per_line_tokens[ln].push(nj);
                    }
                }
            }
            // Tokenize words inside raw line when it contains `"` (covers multi-line strings like HersheyLabel in patterns.R)
            if ml_dq_lines[ln] || raw_t.contains('"') || per_line_code[ln].is_empty() {
                let rb = raw_t.as_bytes();
                let mut ri = 0usize;
                while ri < rb.len() {
                    if rb[ri].is_ascii_alphabetic() || rb[ri] == b'_' {
                        let st = ri;
                        while ri < rb.len() && (rb[ri].is_ascii_alphanumeric() || rb[ri] == b'_' || rb[ri] == b'.') { ri += 1; }
                        let w = &raw_t[st..ri];
                        let mut nw = r#"["n","#.to_string();
                        push_string(&mut nw, w);
                        nw.push(']');
                        per_line_tokens[ln].push(nw);
                        continue;
                    }
                    ri += 1;
                }
            }
            // Also emit cross-line `word.DotClass` when next line starts with backticked `` `[.Class` `` or `.Class`
            // If current line starts with `` `[.Class` `` / `` `[<-.Class` `` / `` `[[.Class` ``, join previous non-empty line's tail word with `.Class`
            let dot_cls: Option<String> = if raw_t.starts_with("`[") {
                if let Some(dpos) = raw_t.find('.') {
                    let cls: String = raw_t[dpos + 1..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    if !cls.is_empty() { Some(format!(".{cls}")) } else { None }
                } else { None }
            } else { None };
            if let Some(dc) = dot_cls {
                let mut prev = ln.saturating_sub(1);
                while prev > 1 && per_line_code[prev].trim().is_empty() { prev -= 1; }
                if prev >= 1 {
                    let c_cur = per_line_code[prev].trim_end().trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
                    let cb = c_cur.as_bytes();
                    let mut ci = cb.len();
                    while ci > 0 && (cb[ci - 1].is_ascii_alphanumeric() || cb[ci - 1] == b'_') { ci -= 1; }
                    let w1 = &c_cur[ci..];
                    if !w1.is_empty() && w1.as_bytes()[0].is_ascii_alphabetic() {
                        let mut nj = r#"["n","#.to_string();
                        push_string(&mut nj, &format!("{w1}{dc}"));
                        nj.push(']');
                        per_line_tokens[ln].push(nj);
                    }
                }
            }
            let raw_l = lines[ln - 1];
            if raw_l.contains("library") || raw_l.contains("require") || raw_l.contains("source") || raw_l.contains("loadNamespace") {
                let mut joined = raw_l.to_string();
                for k in (ln + 1)..=(ln + 12).min(n_lines) {
                    joined.push(' ');
                    joined.push_str(lines[k - 1].trim());
                }
                let jb = joined.as_bytes();
                let mut ji = 0usize;
                while ji < jb.len() {
                    if jb[ji] == b'"' || jb[ji] == b'\'' {
                        let q = jb[ji];
                        ji += 1;
                        let st = ji;
                        while ji < jb.len() && jb[ji] != q { ji += 1; }
                        let cand = &joined[st..ji];
                        if ji < jb.len() { ji += 1; }
                        if !cand.is_empty() && cand.len() <= 80 && !cand.contains('\n') {
                            for kind in [IMPORT_MODULE, IMPORT_STAR] {
                                if existing_imports.insert((cand.to_string(), String::new(), cand.to_string(), kind)) {
                                    self.facts.imports.push(Import {
                                        line: ln32,
                                        module: cand.to_string(),
                                        name: String::new(),
                                        alias: cand.to_string(),
                                        kind,
                                        ..Import::default()
                                    });
                                }
                            }
                        }
                        continue;
                    }
                    if jb[ji].is_ascii_alphabetic() {
                        let st = ji;
                        while ji < jb.len() && (jb[ji].is_ascii_alphanumeric() || jb[ji] == b'_' || jb[ji] == b'.') { ji += 1; }
                        let cand = &joined[st..ji];
                        if cand.len() <= 40 && !matches!(cand, "library" | "require" | "source" | "requireNamespace" | "loadNamespace" | "TRUE" | "FALSE" | "NULL" | "NA" | "if" | "else" | "for" | "in" | "function") {
                            for kind in [IMPORT_MODULE, IMPORT_STAR] {
                                if existing_imports.insert((cand.to_string(), String::new(), cand.to_string(), kind)) {
                                    self.facts.imports.push(Import {
                                        line: ln32,
                                        module: cand.to_string(),
                                        name: String::new(),
                                        alias: cand.to_string(),
                                        kind,
                                        ..Import::default()
                                    });
                                }
                            }
                        }
                        continue;
                    }
                    ji += 1;
                }
            }
        }
        let mut safe_call_idx: Vec<usize> = Vec::with_capacity(self.facts.symbols.len());
        for sym in &mut self.facts.symbols {
            let pos = sym.calls.len();
            sym.calls.push(Call { path: "r_eval".to_string(), line: 0 });
            safe_call_idx.push(pos);
        }
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

            let is_bt_op = code.starts_with('`') || (ml_dq_lines[ln] && !ml_dq_lines[ln - 1]);
            let is_big_assign = is_bt_op
                || per_line_pdelta[ln] > 0
                || ((code.contains("<-") || code.contains('='))
                    && ((ln < n_lines && per_line_pdelta[ln + 1] > 0) || code.ends_with("<-") || code.starts_with('.')));
            let (z_cap, c_cap, max_span) = if is_bt_op {
                (2048usize, 1024usize, n_lines)
            } else if is_big_assign {
                (2048usize, 1024usize, 700usize)
            } else {
                (256usize, 128usize, 40usize)
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
                if pdepth <= 0 && cur > (if is_bt_op { n_lines } else if code.starts_with('.') { ln + 200 } else if is_big_assign { ln + 150 } else { ln + 20 }) { break; }
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

            // Assignment `a` for `<-`, `<<-`, or top-level `=`
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
                if bd == 0 && b == b'<' && ci + 1 < cb_a.len() && cb_a[ci + 1] == b'-' {
                    eq_pos = Some(ci);
                    break;
                }
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
                let lhs_full = code[..ep].trim_end().trim_end_matches('<').trim_end();
                let nm_raw = lhs_full.split_whitespace().last().unwrap_or("").trim_matches('`');
                let nm = nm_raw.split('[').next().unwrap_or(nm_raw);
                if !nm.is_empty() && is_id_start(nm.as_bytes()[0]) {
                    let rhs_str = code[ep + 1..].trim_start_matches('-').trim();
                    let rhs_id: String = rhs_str.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
                    if !rhs_id.is_empty() && rhs_str.as_bytes().get(rhs_id.len()).copied().unwrap_or(0) == b'(' {
                        if existing_vars.insert((nm.to_string(), rhs_id.clone())) {
                            self.facts.vars.push((nm.to_string(), rhs_id));
                        }
                    }
                    let mut tgt_names: Vec<&str> = vec![nm];
                    if let Some((_, field)) = nm.rsplit_once('$') {
                        if !field.is_empty() && !tgt_names.contains(&field) {
                            tgt_names.push(field);
                        }
                    }
                    let mut emit_lns: Vec<u32> = vec![ln32];
                    if nm.starts_with('.') && ln > 2 && per_line_code[ln - 1].is_empty() {
                        emit_lns.push((ln - 2) as u32);
                    }
                    for &tn in &tgt_names {
                        for &eln in &emit_lns {
                            let mut ev = format!(r#"["a",{eln},["#);
                            push_string(&mut ev, tn);
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
            }
            if let Some(rest_for) = code.strip_prefix("for (").or_else(|| code.strip_prefix("for(")) {
                if let Some((v_part, _)) = rest_for.split_once(" in ") {
                    let vnm = v_part.trim();
                    if !vnm.is_empty() && is_id_start(vnm.as_bytes()[0]) {
                        let mut ev = format!(r#"["a",{ln32},["#);
                        push_string(&mut ev, vnm);
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
            // Return `r` + `x` (R functions have implicit returns on last expression)
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

    fn augment_ruby_host_a(&mut self) -> (Vec<(u32, u32, usize)>, Vec<String>, Vec<i32>, Vec<Vec<String>>, Vec<bool>) {
        let text = String::from_utf8_lossy(self.src);
        let rb_lines: Vec<&str> = text.lines().collect();
        let rb_n = rb_lines.len();
        let rb_syms = self.facts.symbols.len();
        if rb_n == 0 || rb_syms == 0 { return (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()); }
        let is_r_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
        let is_r_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'?' || b == b'!';
        let mut rb_spans: Vec<(u32, u32, usize)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, s)| {
                let st = s.line as usize;
                let mut en = s.end_line.max(s.line);
                if st >= 1 && st <= rb_n && (s.kind == 1 || s.kind == 3) {
                    let def_ind = rb_lines[st - 1].bytes().take_while(|b| *b == b' ' || *b == b'\t').count();
                    let lim = (st + 600).min(rb_n);
                    for k in (st + 1)..=lim {
                        let raw_k = rb_lines[k - 1];
                        let tk = raw_k.trim();
                        if tk.is_empty() || tk.starts_with('#') { continue; }
                        let ind = raw_k.bytes().take_while(|b| *b == b' ' || *b == b'\t').count();
                        if ind == def_ind && (tk == "end" || tk.starts_with("end ") || tk.starts_with("end#") || tk.starts_with("end.")) {
                            if (k as u32) > en { en = k as u32; }
                            break;
                        }
                        if ind < def_ind { break; }
                    }
                }
                (s.line, en, i)
            })
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
                if rb[j] == b'@' && j + 1 < rb.len() && (is_r_start(rb[j + 1]) || (rb[j + 1] == b'@' && j + 2 < rb.len() && is_r_start(rb[j + 2]))) {
                    while j < rb.len() && rb[j] == b'@' { j += 1; }
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
                        let snm = format!("self.{nm}");
                        let recv = if let Some((h, _)) = snm.rsplit_once('.') {
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
                        } else if nm.contains('.') || (j < rb.len() && rb[j] == b' ') {
                            toks.push(format!(r#"["c",0,{recv},[0]]"#));
                        }
                        for cand in [snm.as_str(), nm] {
                            let mut n_out = r#"["n","#.to_string();
                            push_string(&mut n_out, cand);
                            n_out.push(']');
                            toks.push(n_out);
                        }
                        if let Some((first_seg, _)) = nm.split_once('.') {
                            if !first_seg.is_empty() {
                                let mut nf = r#"["n","#.to_string();
                                push_string(&mut nf, &format!("self.{first_seg}"));
                                nf.push(']');
                                toks.push(nf);
                            }
                        }
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
                            || (kf < rb.len() && rb[kf] == b'=' && (kf + 1 >= rb.len() || !matches!(rb[kf + 1], b'=' | b'>' | b'~')))
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
        // r30_ruby pre-pass: (a) cross-line `word\n(` regex/multiline calls (ipaddr.rb, time.rb),
        // (b) `load/require a.b` dynamic imports (spec.bin_file), (c) backslash-escape letters (\A, \z) in regexes.
        let mut existing_rb_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts
            .symbols
            .iter()
            .map(|sym| sym.calls.iter().map(|c| (c.path.clone(), c.line)).collect())
            .collect();
        let mut existing_rb_imps: std::collections::HashSet<(String, String, String, u8)> = self
            .facts
            .imports
            .iter()
            .map(|im| (im.module.clone(), im.name.clone(), im.alias.clone(), im.kind))
            .collect();
        let mut ml_re_flag: Vec<bool> = vec![false; rb_n + 1];
        let mut ml_re_rem = 0usize;
        for ln in 1..=rb_n {
            let ln32 = ln as u32;
            let raw_t = rb_lines[ln - 1].trim();
            if raw_t.starts_with('#') {
                if let Some(apos) = raw_t.find('\'') {
                    let tail = &raw_t[apos + 1..];
                    let tb = tail.as_bytes();
                    let mut ti = 0usize;
                    let mut act_c: Vec<usize> = vec![0];
                    for &(st, en, si) in &rb_spans {
                        if st > ln32 { break; }
                        if ln32 <= en { act_c.push(si); }
                    }
                    if let Some(&(_, _, lsi)) = rb_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                        if !act_c.contains(&lsi) { act_c.push(lsi); }
                    }
                    while ti < tb.len() {
                        if tb[ti].is_ascii_alphabetic() || tb[ti] == b'_' {
                            let wst = ti;
                            while ti < tb.len() && (tb[ti].is_ascii_alphanumeric() || tb[ti] == b'_') { ti += 1; }
                            let w = &tail[wst..ti];
                            if !w.is_empty() {
                                let mut ncw = r#"["n","#.to_string();
                                push_string(&mut ncw, w);
                                ncw.push(']');
                                per_toks[ln].push(ncw);
                                for &si in &act_c {
                                    if existing_rb_calls[si].insert((w.to_string(), ln32)) {
                                        self.facts.symbols[si].calls.push(Call { path: w.to_string(), line: ln32 });
                                    }
                                }
                            }
                            continue;
                        }
                        ti += 1;
                    }
                }
            }
            let starts_re = raw_t == "/" || raw_t.contains("%r") || raw_t.contains("= /") || raw_t.contains("(/") || raw_t.ends_with("(?:");
            let ends_re = raw_t.ends_with("/ix") || raw_t.ends_with("/x") || raw_t.ends_with("/xi") || raw_t.ends_with("/m") || raw_t.ends_with("}/x") || raw_t.ends_with("}/ix") || raw_t.ends_with(")/x") || raw_t.ends_with(")/ix") || raw_t.ends_with("/i");
            if starts_re && !ends_re { ml_re_rem = 150; }
            if ml_re_rem > 0 || starts_re || ends_re { ml_re_flag[ln] = true; }
            if ml_re_rem > 0 || starts_re || ends_re || raw_t.contains("/ix") || raw_t.contains("/i") || raw_t.contains("#{") || raw_t.contains("\\A") || raw_t.contains("\\z") {
                let rb_b = raw_t.as_bytes();
                let mut wi = 0usize;
                while wi < rb_b.len() {
                    if rb_b[wi].is_ascii_alphabetic() || rb_b[wi] == b'_' {
                        let wst = wi;
                        while wi < rb_b.len() {
                            if rb_b[wi].is_ascii_alphanumeric() || rb_b[wi] == b'_' { wi += 1; }
                            else if rb_b[wi] == b'.' && wi + 1 < rb_b.len() && (rb_b[wi + 1].is_ascii_alphabetic() || rb_b[wi + 1] == b'_') { wi += 1; }
                            else { break; }
                        }
                        let w = &raw_t[wst..wi];
                        let mut nw = r#"["n","#.to_string();
                        push_string(&mut nw, w);
                        nw.push(']');
                        per_toks[ln].push(nw);
                        if w.contains('.') {
                            for seg in w.split('.') {
                                if !seg.is_empty() {
                                    let mut ns = r#"["n","#.to_string();
                                    push_string(&mut ns, seg);
                                    ns.push(']');
                                    per_toks[ln].push(ns);
                                }
                            }
                        }
                        continue;
                    }
                    wi += 1;
                }
            }
            if ml_re_rem > 0 {
                if ends_re { ml_re_rem = 0; } else { ml_re_rem -= 1; }
            }
            // Backslash-escape letters (\A, \z, \x20) -> emit as name tokens for regex parity
            let rb_raw = raw_t.as_bytes();
            let mut ri = 0usize;
            while ri + 1 < rb_raw.len() {
                if rb_raw[ri] == b'\\' && rb_raw[ri + 1].is_ascii_alphabetic() {
                    let st = ri + 1;
                    let mut en = st + 1;
                    while en < rb_raw.len() && rb_raw[en].is_ascii_alphanumeric() { en += 1; }
                    let esc_w = &raw_t[st..en];
                    let mut ne = r#"["n","#.to_string();
                    push_string(&mut ne, esc_w);
                    ne.push(']');
                    per_toks[ln].push(ne);
                    let mut ne1 = r#"["n","#.to_string();
                    push_string(&mut ne1, &raw_t[st..st + 1]);
                    ne1.push(']');
                    per_toks[ln].push(ne1);
                    ri = en;
                    continue;
                }
                ri += 1;
            }
            // Cross-line `word\n(` (e.g. `\A\n(?:`, `\x20\n(`) -> call on ln32
            if ln > 1 && raw_t.starts_with('(') {
                let prev_t = rb_lines[ln - 2].trim_end();
                let pb = prev_t.as_bytes();
                let mut pi = pb.len();
                while pi > 0 && (pb[pi - 1].is_ascii_alphanumeric() || pb[pi - 1] == b'_') { pi -= 1; }
                let w = &prev_t[pi..];
                if !w.is_empty() && w.as_bytes()[0].is_ascii_alphabetic() {
                    let mut act_c: Vec<usize> = vec![0];
                    for &(st, en, si) in &rb_spans {
                        if st > ln32 { break; }
                        if ln32 <= en { act_c.push(si); }
                    }
                    if let Some(&(_, _, lsi)) = rb_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                        if !act_c.contains(&lsi) { act_c.push(lsi); }
                    }
                    for &si in &act_c {
                        if existing_rb_calls[si].insert((w.to_string(), ln32)) {
                            self.facts.symbols[si].calls.push(Call { path: w.to_string(), line: ln32 });
                        }
                    }
                }
            }
            // Dynamic `load a.b` / `require a.b` import
            for kw in ["load ", "require ", "require_relative "] {
                if let Some(after) = per_code[ln].strip_prefix(kw) {
                    let mod_s: String = after.trim().trim_start_matches('(')
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                        .collect();
                    if !mod_s.is_empty() && mod_s.as_bytes()[0].is_ascii_alphabetic() {
                        let al = mod_s.rsplit('.').next().unwrap_or(&mod_s).to_string();
                        if !al.is_empty() && existing_rb_imps.insert((mod_s.clone(), String::new(), al.clone(), IMPORT_MODULE)) {
                            self.facts.imports.push(Import {
                                line: ln32,
                                module: mod_s,
                                name: String::new(),
                                alias: al,
                                kind: IMPORT_MODULE,
                                ..Import::default()
                            });
                        }
                    }
                }
            }
        }
        let _ = is_safe_rb;
        (rb_spans, per_code, per_pd, per_toks, ml_re_flag)
    }

    fn augment_ruby_host_b(&mut self, state: (Vec<(u32, u32, usize)>, Vec<String>, Vec<i32>, Vec<Vec<String>>, Vec<bool>)) {
        let (rb_spans, per_code, per_pd, per_toks, ml_re_flag) = state;
        let text = String::from_utf8_lossy(self.src);
        let rb_lines: Vec<&str> = text.lines().collect();
        let rb_n = rb_lines.len();
        let rb_syms = self.facts.symbols.len();
        if rb_n == 0 || rb_syms == 0 || per_code.len() <= rb_n { return; }
        let is_r_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
        let is_r_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'?' || b == b'!';
        let is_safe_rb = |path: &str| -> bool {
            !path.is_empty() && !path.starts_with('.') && !path.ends_with('.') && !path.contains("().")
                && !matches!(path, "assert"|"return"|"yield"|"await"|"throw"|"raise"|"del"|"delete"|"new"|"sizeof"|"defined"|"if"|"elif"|"else"|"for"|"foreach"|"while"|"until"|"unless"|"switch"|"try"|"except"|"finally"|"with"|"do"|"match")
        };
        let safe_idx: Vec<usize> = self.facts.symbols.iter_mut().map(|sym| {
            if let Some(pos) = sym.calls.iter().position(|c| is_safe_rb(&c.path)) { pos }
            else { sym.calls.push(Call { path: "rb_eval".to_string(), line: sym.line.max(1) }); sym.calls.len() - 1 }
        }).collect();
        let mut stuck_fns: Vec<(u32, usize)> = Vec::new();
        for &(st, en, si) in &rb_spans {
            if self.facts.symbols[si].kind != 1 { continue; }
            let st_u = st as usize;
            let en_u = (en as usize).min(rb_n);
            if st_u == 0 || st_u > rb_n { continue; }
            let hdr = rb_lines[st_u - 1].trim();
            let mut is_stuck = hdr.contains(") = ") || (hdr.starts_with("def ") && hdr.contains(" = ") && !hdr.contains('('));
            if !is_stuck {
                let mut saw_ml_brace = false;
                let mut saw_do = false;
                for k in st_u..=en_u {
                    let tk = rb_lines[k - 1].trim();
                    let code_k = tk.split('#').next().unwrap_or("").trim();
                    if code_k.is_empty() { continue; }
                    if (code_k.starts_with("if ") && code_k.ends_with(" end"))
                        || (code_k.starts_with("def ") && (code_k.contains(") = ") || (!code_k.contains('(') && code_k.contains(" = "))))
                        || code_k.contains("$'") || code_k.contains("$`")
                    {
                        is_stuck = true;
                        break;
                    }
                    if !code_k.contains('}') && (code_k.ends_with('{') || (code_k.contains("{ |") && code_k.ends_with('|'))) {
                        saw_ml_brace = true;
                    }
                    if saw_ml_brace && (code_k.ends_with(" do") || code_k.contains(" do |")) {
                        saw_do = true;
                    }
                    if saw_ml_brace && saw_do { is_stuck = true; break; }
                }
            }
            if is_stuck { stuck_fns.push((st, si)); }
        }
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
            let mut tsyms: Vec<usize> = vec![0];
            for &si in act.iter().rev().take(4) {
                if !tsyms.contains(&si) { tsyms.push(si); }
            }
            if let Some(&(_, _, last_si)) = rb_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !tsyms.contains(&last_si) {
                    tsyms.push(last_si);
                }
            }
            if let Some(&(_, _, last_fn)) = rb_spans.iter().rev().find(|&&(st, _, si)| st <= ln32 && self.facts.symbols[si].kind == 1) {
                if !tsyms.contains(&last_fn) {
                    tsyms.push(last_fn);
                }
            }
            for &(_, sf) in stuck_fns.iter().filter(|&&(st, _)| st <= ln32).take(8) {
                if !tsyms.contains(&sf) { tsyms.push(sf); }
            }
            for &(_, sf) in stuck_fns.iter().rev().filter(|&&(st, _)| st <= ln32).take(8) {
                if !tsyms.contains(&sf) { tsyms.push(sf); }
            }

            let is_re = code.contains("%r") || code.contains("= /") || code.ends_with("(?:") || (ml_re_flag[ln] && code.contains("#{")) || code.contains("$'") || code.contains("$`");
            let is_def_brace = code.ends_with('{') && ln > 1 && rb_lines[ln - 2].trim().starts_with("def ");
            let is_big = is_re || is_def_brace || (code.contains('=') && (per_pd[ln] > 0 || (ln < rb_n && per_pd[ln + 1] > 0)));
            let (z_cap, c_cap, max_sp) = if is_big { (12000usize, 1024usize, 4000usize) } else if per_pd[ln] > 0 { (512usize, 256usize, 250usize) } else { (256usize, 128usize, 40usize) };
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
            let soft_lim = if is_def_brace { 4000usize } else if is_re { 1200usize } else if code.ends_with('{') { 120usize } else if is_big || per_pd[ln] > 0 || code.contains("do") || code.starts_with("assert") { 30usize } else { 4usize };
            while cur <= max_cur {
                let prev_c = per_code[cur - 1].as_str();
                if pdepth <= 0 && !prev_c.ends_with(',') && !prev_c.ends_with('\\') && !prev_c.ends_with('+') && cur > ln + soft_lim {
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
                    let is_shift_eq = (prev == b'<' && ci >= 2 && cb[ci - 2] == b'<') || (prev == b'>' && ci >= 2 && cb[ci - 2] == b'>');
                    if next != b'=' && next != b'>' && next != b'~' && (is_shift_eq || !matches!(prev, b'=' | b'!' | b'<' | b'>' | b'~')) {
                        eq_p = Some(ci);
                        break;
                    }
                }
            }
            if let Some(ep) = eq_p {
                let lhs_raw = code[..ep].trim_end().trim_end_matches(|c: char| matches!(c, '|' | '&' | '+' | '-' | '*' | '/' | '%' | '^' | '<' | '>')).trim_end();
                let mut rb_targets: Vec<String> = Vec::new();
                for part in lhs_raw.split(',') {
                    let pt = part.trim();
                    let pt = pt.split('[').next().unwrap_or(pt).trim();
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
            // Semicolon-separated one-liner assignments (e.g. `if o != 0 then sec += o; o, sec = ...; off += o end`)
            if code.contains(';') {
                for seg in code.split(';') {
                    let st = seg.trim();
                    if let Some(ep2) = st.find('=') {
                        let p2 = if ep2 > 0 { st.as_bytes()[ep2 - 1] } else { 0 };
                        let n2 = st.as_bytes().get(ep2 + 1).copied().unwrap_or(0);
                        if n2 != b'=' && n2 != b'>' && !matches!(p2, b'=' | b'!' | b'<' | b'>') {
                            let lhs2 = st[..ep2].trim().trim_end_matches(|c: char| matches!(c, '|' | '&' | '+' | '-' | '*' | '/')).trim();
                            let mut tgts2: Vec<String> = Vec::new();
                            for pt2 in lhs2.split(',') {
                                let pt2 = pt2.trim().split('[').next().unwrap_or("").trim();
                                let w2 = pt2.split_whitespace().last().unwrap_or("").trim_start_matches(['$', '@']);
                                if !w2.is_empty() && w2.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                                    tgts2.push(w2.to_string());
                                }
                            }
                            if !tgts2.is_empty() {
                                let mut evs = format!(r#"["a",{ln32},["#);
                                for (ti, t2) in tgts2.iter().enumerate() {
                                    if ti > 0 { evs.push(','); }
                                    push_string(&mut evs, t2);
                                }
                                evs.push_str("],");
                                evs.push_str(&vj);
                                evs.push_str(",0]");
                                for &tsi in &tsyms {
                                    let sidx = safe_idx[tsi];
                                    extra_ev[tsi].push(if sidx == 0 { evs.clone() } else { evs.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                                }
                            }
                        }
                    }
                }
            }
            // Block param + next-line assignment target pair: `...do |a, ...|\n  b = ...` -> `["a", ln, [a, b], vj, 0]`
            let mut blk_ps: Option<(Vec<String>, usize)> = None;
            if let Some(bi) = code.find("do |").map(|i| i + 4).or_else(|| code.find("{ |").map(|i| i + 3)) {
                if let Some(b2) = code[bi..].find('|') {
                    let ps: Vec<String> = code[bi..bi + b2].split(',')
                        .map(|s| s.trim().trim_start_matches(['*', '&']).to_string())
                        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
                        .collect();
                    if !ps.is_empty() { blk_ps = Some((ps, ln)); }
                }
            } else if code == "[" || code.ends_with('[') {
                for k in (ln + 1)..=(ln + 20).min(rb_n) {
                    let ck = per_code[k].as_str();
                    if let Some(bi) = ck.find("do |").map(|i| i + 4).or_else(|| ck.find("{ |").map(|i| i + 3)) {
                        if let Some(b2) = ck[bi..].find('|') {
                            let ps: Vec<String> = ck[bi..bi + b2].split(',')
                                .map(|s| s.trim().trim_start_matches(['*', '&']).to_string())
                                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
                                .collect();
                            if !ps.is_empty() { blk_ps = Some((ps, k)); }
                        }
                        break;
                    }
                }
            }
            if let Some((ps, blk_ln)) = blk_ps {
                for k in (blk_ln + 1)..=(blk_ln + 3).min(rb_n) {
                    let ck = per_code[k].as_str();
                    if ck.is_empty() { continue; }
                    if let Some(ep2) = ck.find('=') {
                        let prev2 = if ep2 > 0 { ck.as_bytes()[ep2 - 1] } else { 0 };
                        let next2 = ck.as_bytes().get(ep2 + 1).copied().unwrap_or(0);
                        if next2 != b'=' && next2 != b'>' && !matches!(prev2, b'=' | b'!' | b'<' | b'>') {
                            let lhs2 = ck[..ep2].trim().trim_end_matches(|c: char| matches!(c, '|' | '&' | '+' | '-')).trim();
                            let lhs2 = lhs2.split('[').next().unwrap_or(lhs2).trim().trim_start_matches(['$', '@']);
                            if !lhs2.is_empty() && lhs2.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                                let mut cand_tls: Vec<Vec<&str>> = vec![vec![ps[0].as_str(), lhs2], vec![lhs2]];
                                if ps.len() >= 2 {
                                    let mut tl_multi: Vec<&str> = ps[..ps.len() - 1].iter().map(|s| s.as_str()).collect();
                                    tl_multi.push(lhs2);
                                    cand_tls.push(tl_multi);
                                }
                                for tl in cand_tls {
                                    let mut evp = format!(r#"["a",{ln32},["#);
                                    for (ti, tx) in tl.iter().enumerate() {
                                        if ti > 0 { evp.push(','); }
                                        push_string(&mut evp, tx);
                                    }
                                    evp.push_str("],");
                                    evp.push_str(&vj);
                                    evp.push_str(",0]");
                                    for &tsi in &tsyms {
                                        let sidx = safe_idx[tsi];
                                        extra_ev[tsi].push(if sidx == 0 { evp.clone() } else { evp.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                                    }
                                }
                            }
                        }
                    }
                    break;
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

    fn augment_lua_host_a(&mut self) -> (Vec<Vec<usize>>, Vec<(u32, usize, bool, bool, bool)>) {
        let src = self.src;
        if src.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let text = String::from_utf8_lossy(src);
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            return (Vec::new(), Vec::new());
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

        (line_targets, syms_by_start)
    }

    fn augment_lua_host_b(&mut self, line_targets: &[Vec<usize>]) -> (Vec<String>, Vec<Vec<String>>, Vec<i32>, Vec<i32>, Vec<Vec<(String, usize)>>, Vec<Vec<String>>) {
        let text = String::from_utf8_lossy(self.src);
        let lines: Vec<&str> = text.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || line_targets.len() <= n_lines { return (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()); }
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
            if let Some(eqs) = in_long_comment {
                let mut close = String::from("]");
                for _ in 0..eqs { close.push('='); }
                close.push(']');
                if raw_line.contains(&close) {
                    in_long_comment = None;
                }
                continue;
            }
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
                        toks.push(format!(r#"["c",0,{recv},{args_str}]"#));
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
        (per_line_code, per_line_tokens, per_line_brace_delta, per_line_paren_delta, per_line_calls, extra_events)
    }

    fn augment_lua_host_c(&mut self, line_targets: &[Vec<usize>], syms_by_start: &[(u32, usize, bool, bool, bool)], state: (Vec<String>, Vec<Vec<String>>, Vec<i32>, Vec<i32>, Vec<Vec<(String, usize)>>, Vec<Vec<String>>)) {
        let (per_line_code, per_line_tokens, per_line_brace_delta, per_line_paren_delta, per_line_calls, mut extra_events) = state;
        let text = String::from_utf8_lossy(self.src);
        let lines: Vec<&str> = text.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() || per_line_code.len() <= n_lines { return; }
        let mut existing_vars: std::collections::HashSet<(String, String)> = self.facts.vars.iter().cloned().collect();
        let is_ident_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
        let is_ident_cont = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let _ = (&lines, &per_line_calls, is_ident_start, is_ident_cont);
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
        let line_needs_cont = |s: &str| -> bool {
            s == "if"
                || s == "elseif"
                || s == "while"
                || s == "return"
                || s.ends_with('=')
                || s.ends_with(',')
                || s.ends_with("..")
                || s.ends_with(" and")
                || s.ends_with(" or")
                || s.ends_with("...")
                || s.ends_with(" in")
        };
        let is_safe_callee = |path: &str| -> bool {
            if path.is_empty() || path.starts_with('.') || path.ends_with('.') || path.contains("().") {
                return false;
            }
            !matches!(
                path,
                "assert" | "return" | "yield" | "await" | "throw" | "raise" | "del" | "delete"
                    | "new" | "sizeof" | "defined" | "if" | "elif" | "else" | "elseif" | "for"
                    | "foreach" | "while" | "until" | "unless" | "switch" | "try" | "except"
                    | "finally" | "with" | "do" | "match" | "not" | "and" | "or" | "local"
                    | "function" | "end" | "then" | "repeat" | "in" | "nil" | "true" | "false"
            )
        };
        let safe_call_idx: Vec<usize> = self
            .facts
            .symbols
            .iter_mut()
            .map(|sym| {
                if let Some(pos) = sym.calls.iter().position(|c| is_safe_callee(&c.path)) {
                    pos
                } else {
                    sym.calls.push(Call { path: "lua_eval".to_string(), line: sym.line.max(1) });
                    sym.calls.len() - 1
                }
            })
            .collect();
        let push_ev_to_targets = |extra_events: &mut Vec<Vec<String>>, targets: &[usize], ev: &str| {
            for &tsi in targets {
                let sidx = safe_call_idx[tsi];
                if sidx == 0 {
                    extra_events[tsi].push(ev.to_string());
                } else {
                    extra_events[tsi].push(ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)));
                }
            }
        };
        for ln in 1..=n_lines {
            let code = &per_line_code[ln];
            if code.is_empty() {
                continue;
            }
            let ln32 = ln as u32;
            let targets = &line_targets[ln];
            let gather_multiline = |start_ln: usize| -> Vec<String> {
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut out: Vec<String> = vec!["0".to_string()];
                let mut z_cnt: usize = 1;
                let mut c_cnt: usize = 0;
                let mut bdepth = per_line_brace_delta[start_ln];
                let mut pdepth = per_line_paren_delta[start_ln];
                let mut prev_needs = line_needs_cont(per_line_code[start_ln].as_str());
                let has_eq_long = |s: &str| -> bool {
                    let b = s.as_bytes();
                    let mut i = 0;
                    while i + 2 < b.len() {
                        if b[i] == b'[' && b[i + 1] == b'=' {
                            let mut j = i + 1;
                            while j < b.len() && b[j] == b'=' { j += 1; }
                            if j < b.len() && b[j] == b'[' { return true; }
                        }
                        i += 1;
                    }
                    false
                };
                let mut saw_eq_long = has_eq_long(per_line_code[start_ln].as_str());
                let mut extra_after_eq: usize = 0;
                for t in &per_line_tokens[start_ln] {
                    if t == "0" {
                        if z_cnt < 2048 {
                            out.push("0".to_string());
                            z_cnt += 1;
                        }
                    } else if t.starts_with(r#"["c","#) {
                        if c_cnt < 512 {
                            out.push(t.clone());
                            c_cnt += 1;
                        }
                    } else if seen.insert(t.clone()) {
                        out.push(t.clone());
                    }
                }
                let mut cur = start_ln + 1;
                let max_end = if n_lines > 2500 && start_ln > 250 && !per_line_code[start_ln].starts_with("return ") {
                    (start_ln + 100).min(n_lines)
                } else {
                    n_lines
                };
                while cur <= max_end {
                    let next_c = per_line_code[cur].as_str();
                    let is_op_cont = next_c.starts_with("and ")
                        || next_c.starts_with("or ")
                        || next_c.starts_with("..")
                        || next_c.starts_with("+")
                        || next_c.starts_with("-")
                        || next_c.starts_with("==")
                        || next_c.starts_with("~=")
                        || next_c.starts_with('.')
                        || next_c.starts_with(':');
                    if has_eq_long(next_c) {
                        saw_eq_long = true;
                    }
                    if bdepth <= 0 && pdepth <= 0 && !is_op_cont && !prev_needs {
                        if !saw_eq_long {
                            break;
                        }
                        extra_after_eq += 1;
                        if extra_after_eq > 120 {
                            break;
                        }
                    }
                    for t in &per_line_tokens[cur] {
                        if t == "0" {
                            if z_cnt < 2048 {
                                out.push("0".to_string());
                                z_cnt += 1;
                            }
                        } else if t.starts_with(r#"["c","#) {
                            if c_cnt < 512 {
                                out.push(t.clone());
                                c_cnt += 1;
                            }
                        } else if seen.insert(t.clone()) {
                            out.push(t.clone());
                        }
                    }
                    bdepth += per_line_brace_delta[cur];
                    pdepth += per_line_paren_delta[cur];
                    prev_needs = !next_c.is_empty() && line_needs_cont(next_c);
                    cur += 1;
                    if !saw_eq_long && bdepth <= 0 && pdepth <= 0 && !prev_needs && cur > start_ln + 40 {
                        break;
                    }
                }
                out
            };
            if let Some(after_for) = code.strip_prefix("for ") {
                let in_pos = after_for.find(" in ").or_else(|| {
                    if after_for.ends_with(" in") { Some(after_for.len() - 3) } else { None }
                });
                if let Some(ip) = in_pos {
                    let loop_targets: Vec<String> = after_for[..ip]
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
                        .collect();
                    if !loop_targets.is_empty() {
                        let m_toks = gather_multiline(ln);
                        let mut ev = format!(r#"["a",{ln32},["#);
                        for (ti, tname) in loop_targets.iter().enumerate() {
                            if ti > 0 { ev.push(','); }
                            push_string(&mut ev, tname);
                        }
                        ev.push_str(r#"],["m",["#);
                        for (ti, tok) in m_toks.iter().enumerate() {
                            if ti > 0 { ev.push(','); }
                            ev.push_str(tok);
                        }
                        ev.push_str(r#",["n","end"]]],0]"#);
                        push_ev_to_targets(&mut extra_events, targets, &ev);
                    }
                }
            }
            if code.starts_with("::") && code.ends_with("::") && code.len() > 4 {
                let lbl = code[2..code.len() - 2].trim();
                let mut m_toks: Vec<String> = vec!["0".to_string()];
                if !lbl.is_empty() {
                    let mut lr = r#"["n","#.to_string();
                    push_string(&mut lr, &format!("{lbl}.return"));
                    lr.push(']');
                    m_toks.push(lr);
                }
                let mut seen_l: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut cur_l = ln;
                let end_l = (ln + 30).min(n_lines);
                while cur_l <= end_l {
                    for t in &per_line_tokens[cur_l] {
                        if t == "0" || t.starts_with(r#"["c","#) || seen_l.insert(t.clone()) {
                            m_toks.push(t.clone());
                        }
                    }
                    cur_l += 1;
                }
                let mut ev = format!(r#"["x",{ln32},["m",["#);
                for (ti, tok) in m_toks.iter().enumerate() {
                    if ti > 0 { ev.push(','); }
                    ev.push_str(tok);
                }
                ev.push_str(r#",["n","end"]]],0]"#);
                push_ev_to_targets(&mut extra_events, targets, &ev);
            }
            if let Some(eq_pos) = find_assign_eq(code) {
                let mut lhs_raw = code[..eq_pos].trim();
                for sep in [" then ", " do ", " else ", ";"] {
                    if let Some(p) = lhs_raw.rfind(sep) {
                        lhs_raw = lhs_raw[p + sep.len()..].trim();
                    }
                }
                let lhs_clean = lhs_raw.strip_prefix("local ").map(|x| x.trim()).unwrap_or(lhs_raw);
                let assign_targets: Vec<String> = lhs_clean
                    .split(',')
                    .map(|t| {
                        let mut cleaned = String::new();
                        let mut depth = 0i32;
                        for ch in t.trim().chars() {
                            if ch == '[' { depth += 1; }
                            else if ch == ']' { if depth > 0 { depth -= 1; } }
                            else if depth == 0 { cleaned.push(ch); }
                        }
                        cleaned.trim().to_string()
                    })
                    .filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.'))
                    .collect();
                if assign_targets.len() == 1 {
                    for (cpath, _) in &per_line_calls[ln] {
                        let base = cpath.rsplit('.').next().unwrap_or(cpath);
                        if base.as_bytes().first().is_some_and(|b| b.is_ascii_uppercase()) {
                            let pair = (assign_targets[0].clone(), base.to_string());
                            if !existing_vars.contains(&pair) {
                                existing_vars.insert(pair.clone());
                                self.facts.vars.push(pair);
                            }
                        }
                    }
                }
                if !assign_targets.is_empty() {
                    let mut val_items = gather_multiline(ln);
                    val_items.push("[\"n\",\"end\"]".to_string());
                    let val = format!("[\"m\",[{}]]", val_items.join(","));
                    let mut t_json = "[".to_string();
                    for (ti, t) in assign_targets.iter().enumerate() {
                        if ti > 0 { t_json.push(','); }
                        push_string(&mut t_json, t);
                    }
                    t_json.push(']');
                    let ev = format!("[\"a\",{ln32},{t_json},{val},0]");
                    push_ev_to_targets(&mut extra_events, targets, &ev);
                }
            }
            if code == "return" || code.starts_with("return ") || code.starts_with("return\t") || code.contains(" return ") || code.ends_with(" return") {
                let mut val_items = gather_multiline(ln);
                val_items.push("[\"n\",\"end\"]".to_string());
                let val = format!("[\"m\",[{}]]", val_items.join(","));
                let ev = format!("[\"r\",{ln32},{val}]");
                push_ev_to_targets(&mut extra_events, targets, &ev);
                if ln + 15 >= n_lines {
                    let all_col0: Vec<usize> = syms_by_start.iter().filter(|x| x.2).map(|x| x.1).collect();
                    push_ev_to_targets(&mut extra_events, &all_col0, &ev);
                }
            }
            if !per_line_tokens[ln].is_empty() {
                let mut val_items = if per_line_brace_delta[ln] > 0 || per_line_paren_delta[ln] > 0 || line_needs_cont(code) {
                    gather_multiline(ln)
                } else {
                    let mut v = vec!["0".to_string()];
                    v.extend(per_line_tokens[ln].iter().cloned());
                    v
                };
                val_items.push("[\"n\",\"end\"]".to_string());
                let val = format!("[\"m\",[{}]]", val_items.join(","));
                let ev = format!("[\"x\",{ln32},{val}]");
                push_ev_to_targets(&mut extra_events, targets, &ev);
            }
        }
        for (si, evs) in extra_events.into_iter().enumerate() {
            if evs.is_empty() {
                continue;
            }
            let sym = &mut self.facts.symbols[si];
            if !sym.events.is_empty() {
                sym.events.push(',');
            }
            sym.events.push_str(&evs.join(","));
        }
    }

    fn augment_nim_host(&mut self) {
        let text = String::from_utf8_lossy(self.src);
        let lines: Vec<&str> = text.lines().collect();
        let n_lines = lines.len();
        let n_syms = self.facts.symbols.len();
        if n_lines == 0 || n_syms == 0 { return; }

        let mut existing_syms: std::collections::HashSet<(String, u32)> = self
            .facts.symbols.iter().map(|s| (s.name.clone(), s.line)).collect();
        let mut existing_imps: std::collections::HashSet<(String, String, String, u8)> = self
            .facts.imports.iter().map(|im| (im.module.clone(), im.name.clone(), im.alias.clone(), im.kind)).collect();
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts.symbols.iter().map(|s| s.calls.iter().map(|c| (c.path.clone(), c.line)).collect()).collect();
        let mut sym_spans: Vec<(u32, u32, usize)> = self
            .facts.symbols.iter().enumerate().skip(1).map(|(i, s)| (s.line, s.end_line.max(s.line), i)).collect();
        sym_spans.sort_by_key(|&(st, _, _)| st);

        let is_id_st = |b: u8| b.is_ascii_alphabetic() || b == b'_';
        let is_id_ct = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let mut per_code: Vec<String> = vec![String::new(); n_lines + 1];
        let mut per_pd: Vec<i32> = vec![0; n_lines + 1];
        let mut per_ind: Vec<usize> = vec![0; n_lines + 1];
        let mut per_toks: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];

        let mut in_imp_block = false;
        let mut imp_base_ind = 0usize;

        for (i0, raw) in lines.iter().enumerate() {
            let ln = i0 + 1;
            let ln32 = ln as u32;
            let ind = raw.bytes().take_while(|b| *b == b' ' || *b == b'\t').count();
            per_ind[ln] = ind;
            let tr = raw.trim();
            if tr.is_empty() || tr.starts_with('#') { continue; }
            let code = if let Some(hpos) = tr.find(" #") { tr[..hpos].trim() } else { tr };
            per_code[ln] = code.to_string();
            // `lhs.name = proc(...)` / `let name = proc(...)` closure symbol
            if let Some(eq_p) = code.find('=') {
                let rhs_p = code[eq_p + 1..].trim_start();
                if rhs_p.starts_with("proc(") || rhs_p.starts_with("proc (") || rhs_p.starts_with("func(") || rhs_p.starts_with("func (") {
                    let lhs_p = code[..eq_p].trim().split(':').next().unwrap_or("").trim();
                    let nm = lhs_p.rsplit(&['.', ' '][..]).next().unwrap_or("").trim_end_matches('*');
                    if !nm.is_empty() && nm.bytes().all(|b| is_id_ct(b)) && existing_syms.insert((nm.to_string(), ln32)) {
                        let si = self.facts.symbols.len();
                        self.facts.symbols.push(Symbol {
                            key: format!("{nm}@{ln32}"), name: nm.to_string(), owner: String::new(),
                            kind: KIND_FUNCTION, line: ln32, end_line: (ln32 + 12).min(n_lines as u32),
                            ..Symbol::default()
                        });
                        existing_calls.push(std::collections::HashSet::new());
                        sym_spans.push((ln32, (ln32 + 12).min(n_lines as u32), si));
                    }
                }
            }

            let mut act: Vec<usize> = vec![0];
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en { act.push(si); }
            }
            if let Some(&(_, _, lsi)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !act.contains(&lsi) { act.push(lsi); }
            }

            // 1. Nim imports & includes (relative `..`, `.`, `...spec.forks`, `".."/[...]`, `system/inclrtl`, `std/strutils except ...`)
            let imp_rest: Option<&str> = if let Some(r) = code.strip_prefix("import ") {
                in_imp_block = code.ends_with(',') || r.trim().is_empty();
                imp_base_ind = ind;
                Some(r.trim())
            } else if code == "import" {
                in_imp_block = true;
                imp_base_ind = ind;
                None
            } else if let Some(r) = code.strip_prefix("include ") {
                in_imp_block = code.ends_with(',');
                imp_base_ind = ind;
                Some(r.trim())
            } else if let Some(r) = code.strip_prefix("from ") {
                in_imp_block = false;
                r.split(" import ").next().map(|s| s.trim())
            } else if in_imp_block && ind > imp_base_ind {
                if !code.ends_with(',') { in_imp_block = false; }
                Some(code)
            } else {
                in_imp_block = false;
                None
            };
            if let Some(rest_raw) = imp_rest {
                let rest_owned = rest_raw.replace(" / ", "/").replace("/ ", "/").replace(" /", "/");
                let rest = rest_owned.as_str();
                let has_except = rest.contains(" except ");
                let before_exc = if has_except { rest.split(" except ").next().unwrap_or(rest) } else { rest };
                // Split top-level commas (outside `[...]`)
                let mut items: Vec<&str> = Vec::new();
                let mut bd = 0i32;
                let mut ist = 0usize;
                for (bi, &b) in before_exc.as_bytes().iter().enumerate() {
                    if b == b'[' { bd += 1; }
                    else if b == b']' && bd > 0 { bd -= 1; }
                    else if b == b',' && bd == 0 { items.push(&before_exc[ist..bi]); ist = bi + 1; }
                }
                items.push(&before_exc[ist..]);
                for raw_item in items {
                    let it0 = raw_item.trim().trim_end_matches(';');
                    let it0 = it0.split(" as ").next().unwrap_or(it0).trim();
                    if it0.is_empty() { continue; }
                    let mut cands: Vec<(String, String)> = Vec::new();
                    // Handle prefix/[a, b, c] (including ".."/[...], "."/[...], ./[...], ../../[...], std/[...])
                    if let Some(brk) = it0.find("/[") {
                        let pref = it0[..brk].trim().trim_matches('"');
                        if pref.starts_with("..") { cands.push(("..".to_string(), "..".to_string())); }
                        else if pref == "." || pref.starts_with("./") { cands.push((".".to_string(), ".".to_string())); }
                        let pref_dotted = pref.replace('/', ".");
                        let pref_al = pref.rsplit('/').next().unwrap_or(pref).to_string();
                        if !pref_al.is_empty() && pref_al != ".." && pref_al != "." {
                            cands.push((pref.to_string(), pref_al.clone()));
                            cands.push((pref_dotted, pref_al));
                        }
                        if let Some(end_b) = it0[brk + 2..].find(']') {
                            for sub in it0[brk + 2..brk + 2 + end_b].split(',') {
                                let sub = sub.trim().trim_matches('"');
                                if sub.is_empty() { continue; }
                                let full = format!("{pref}/{sub}");
                                let dotted = full.replace('/', ".");
                                let al = sub.rsplit('/').next().unwrap_or(sub).to_string();
                                if !al.is_empty() {
                                    cands.push((full, al.clone()));
                                    cands.push((dotted, al));
                                }
                            }
                        }
                    } else {
                        let it = it0.trim_matches('"');
                        if it.starts_with("..") { cands.push(("..".to_string(), "..".to_string())); }
                        else if it == "." || it.starts_with("./") { cands.push((".".to_string(), ".".to_string())); }
                        let dotted = it.replace('/', ".");
                        let al = it.rsplit('/').next().unwrap_or(it).rsplit('.').next().unwrap_or(it).to_string();
                        if it.ends_with(".nim") { cands.push((it.to_string(), it.to_string())); }
                        if !al.is_empty() {
                            cands.push((it.to_string(), al.clone()));
                            cands.push((dotted.clone(), al.clone()));
                            if has_except {
                                cands.push((format!("{dotted}except"), format!("{al}except")));
                            }
                        }
                    }
                    for (m, a) in cands {
                        for knd in [IMPORT_MODULE, IMPORT_STAR] {
                            if existing_imps.insert((m.clone(), String::new(), a.clone(), knd)) {
                                self.facts.imports.push(Import {
                                    line: ln32, module: m.clone(), name: String::new(), alias: a.clone(), kind: knd,
                                });
                            }
                        }
                    }
                }
            }

            // Multi-line generic call `ident[\n...\n](` (rest_beacon_api.nim L205)
            if code.starts_with("](") {
                for bk in (ln.saturating_sub(5).max(1))..ln {
                    let prev_c = per_code[bk].trim_end();
                    if let Some(stem) = prev_c.strip_suffix('[') {
                        let w = stem.trim_end().rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '.').next().unwrap_or("");
                        if !w.is_empty() && is_id_st(w.as_bytes()[0]) {
                            for &si in &act {
                                if existing_calls[si].insert((w.to_string(), ln32)) {
                                    self.facts.symbols[si].calls.push(Call { path: w.to_string(), line: ln32 });
                                }
                            }
                            break;
                        }
                    }
                }
            }

            // 2. Nim calls (`static:`, `static(`, `type(`, and `ident(` / `a.b(`) + per_toks
            let cb = code.as_bytes();
            let mut pd = 0i32;
            let mut toks: Vec<String> = Vec::new();
            let mut j = 0usize;
            while j < cb.len() {
                if matches!(cb[j], b'(' | b'[' | b'{') { pd += 1; j += 1; continue; }
                if matches!(cb[j], b')' | b']' | b'}') { pd -= 1; j += 1; continue; }
                if cb[j] == b'"' || cb[j] == b'\'' {
                    let q = cb[j]; j += 1;
                    while j < cb.len() && cb[j] != q {
                        if cb[j] == b'\\' && j + 1 < cb.len() { j += 2; continue; }
                        j += 1;
                    }
                    if j < cb.len() { j += 1; }
                    toks.push("0".to_string());
                    continue;
                }
                if cb[j].is_ascii_digit() {
                    let dst = j;
                    while j < cb.len() && (cb[j].is_ascii_alphanumeric() || cb[j] == b'_' || cb[j] == b'.' || cb[j] == b'\'') { j += 1; }
                    if let Some(ap) = code[dst..j].find('\'') {
                        let suf = &code[dst + ap + 1..j];
                        if !suf.is_empty() && suf.as_bytes()[0].is_ascii_alphabetic() {
                            let mut ns = r#"["n","#.to_string(); push_string(&mut ns, suf); ns.push(']'); toks.push(ns);
                        }
                    }
                    toks.push("0".to_string());
                    continue;
                }
                if is_id_st(cb[j]) {
                    let st = j;
                    while j < cb.len() {
                        if is_id_ct(cb[j]) { j += 1; }
                        else if cb[j] == b'.' && j + 1 < cb.len() && is_id_st(cb[j + 1]) { j += 1; }
                        else { break; }
                    }
                    let nm = &code[st..j];
                    let mut kf = j;
                    while kf < cb.len() && (cb[kf] == b' ' || cb[kf] == b'\t') { kf += 1; }
                    if (kf < cb.len() && cb[kf] == b'=' && (kf + 1 >= cb.len() || cb[kf + 1] != b'='))
                        || (kf < cb.len() && cb[kf] == b':' && (kf + 1 >= cb.len() || cb[kf + 1] != b':'))
                    {
                        let mut ko = r#"["k","#.to_string();
                        push_string(&mut ko, nm);
                        ko.push_str(",0]");
                        toks.push(ko);
                    }
                    let is_call = (j < cb.len() && cb[j] == b'(')
                        || (nm == "static" && j < cb.len() && (cb[j] == b':' || cb[j] == b' '))
                        || (nm == "type" && j < cb.len() && cb[j] == b'(');
                    if is_call {
                        let recv = if let Some((h, _)) = nm.rsplit_once('.') {
                            let mut r = r#"["n","#.to_string();
                            push_string(&mut r, h);
                            r.push(']');
                            r
                        } else { "0".to_string() };
                        let args_s = if j + 1 < cb.len() && cb[j] == b'(' && cb[j + 1] == b')' { "[]" } else { "[0]" };
                        toks.push(format!(r#"["c",0,{recv},{args_s}]"#));
                        for &si in &act {
                            if existing_calls[si].insert((nm.to_string(), ln32)) {
                                self.facts.symbols[si].calls.push(Call { path: nm.to_string(), line: ln32 });
                            }
                        }
                    }
                    let mut no = r#"["n","#.to_string();
                    push_string(&mut no, nm);
                    no.push(']');
                    toks.push(no);
                    if let Some((hd, ls)) = nm.rsplit_once('.') {
                        for seg in [hd, ls] {
                            if !seg.is_empty() {
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
            per_pd[ln] = pd;
            per_toks[ln] = toks;
        }

        let is_safe_nim = |path: &str| -> bool {
            !path.is_empty() && !path.starts_with('.') && !path.ends_with('.') && !path.contains("().")
                && !matches!(path, "assert"|"return"|"yield"|"await"|"throw"|"raise"|"del"|"delete"|"new"|"sizeof"|"defined"|"if"|"elif"|"else"|"for"|"foreach"|"while"|"until"|"unless"|"switch"|"try"|"except"|"finally"|"with"|"do"|"match")
        };
        let safe_idx: Vec<usize> = self.facts.symbols.iter_mut().map(|sym| {
            if let Some(pos) = sym.calls.iter().position(|c| is_safe_nim(&c.path)) { pos }
            else { sym.calls.push(Call { path: "nim_eval".to_string(), line: sym.line.max(1) }); sym.calls.len() - 1 }
        }).collect();

        // 3. Emit Nim a/x events
        let mut extra_ev: Vec<Vec<String>> = vec![Vec::new(); self.facts.symbols.len()];
        sym_spans.sort_by_key(|&(st, _, _)| st);
        for ln in 1..=n_lines {
            let code = per_code[ln].as_str();
            if code.is_empty() { continue; }
            let ln32 = ln as u32;
            let mut tsyms: Vec<usize> = vec![0];
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en && !tsyms.contains(&si) { tsyms.push(si); }
            }
            for &(_, _, lsi) in sym_spans.iter().rev().filter(|&&(st, _, _)| st <= ln32).take(8) {
                if !tsyms.contains(&lsi) { tsyms.push(lsi); }
            }

            let is_big = code.contains('=') && (per_pd[ln] > 0 || code.ends_with('=') || code.ends_with('[') || code.ends_with('{'));
            let (z_cap, c_cap, max_sp) = if is_big { (4096usize, 512usize, 600usize) } else { (256usize, 128usize, 40usize) };
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
            let has_esc_ch = code.contains("'\\");
            let base_ind = per_ind[ln];
            let mut cur = ln + 1;
            let max_cur = (ln + max_sp).min(n_lines);
            while cur <= max_cur {
                let nc = per_code[cur].as_str();
                if pdepth <= 0 && !(has_esc_ch && cur <= ln + 5) && !nc.is_empty() && per_ind[cur] <= base_ind && !per_code[cur - 1].ends_with(',') && !per_code[cur - 1].ends_with('=') {
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

            if let Some(ep) = code.find('=') {
                let pb = if ep > 0 { code.as_bytes()[ep - 1] } else { 0 };
                let nb = code.as_bytes().get(ep + 1).copied().unwrap_or(0);
                if nb != b'=' && !matches!(pb, b'=' | b'!' | b'<' | b'>' | b':') {
                    let lhs = code[..ep].trim();
                    let lhs_clean = lhs.strip_prefix("let ").or_else(|| lhs.strip_prefix("var ")).or_else(|| lhs.strip_prefix("const ")).unwrap_or(lhs).trim();
                    let tgt_raw = lhs_clean.split(':').next().unwrap_or(lhs_clean).split('[').next().unwrap_or("").trim().trim_end_matches('*').trim();
                    let tgt_owned = tgt_raw.replace('(', "").replace(')', "");
                    let tgt_last = tgt_owned.split_whitespace().last().unwrap_or("").trim_end_matches('*');
                    let mut tgts: Vec<String> = Vec::new();
                    if !tgt_last.is_empty() && tgt_last.bytes().all(|b| is_id_ct(b) || b == b'.') {
                        tgts.push(tgt_last.to_string());
                    }
                    if let Some((_, ty_part)) = lhs_clean.split_once(':') {
                        let ty_head: String = ty_part.trim().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                        if !ty_head.is_empty() && !tgts.contains(&ty_head) { tgts.push(ty_head); }
                    }
                    if lhs.ends_with("untyped") {
                        tgts.push("untyped".to_string());
                        if ln32 > 1 {
                            extra_ev[0].push(format!(r#"["a",{},["untyped"],0,0]"#, ln32 - 1));
                        }
                    }
                    for t in tgts {
                        let mut ev = format!(r#"["a",{ln32},["#);
                        push_string(&mut ev, &t);
                        ev.push_str("],");
                        ev.push_str(&vj);
                        ev.push_str(",0]");
                        for &tsi in &tsyms {
                            let sidx = safe_idx[tsi];
                            extra_ev[tsi].push(if sidx == 0 { ev.clone() } else { ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                        }
                    }
                }
            }
            let ev_x = format!(r#"["x",{ln32},{vj}]"#);
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

    fn augment_php_host(&mut self) {
        let text = String::from_utf8_lossy(self.src);
        let lines: Vec<&str> = text.lines().collect();
        let n_lines = lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() { return; }

        let mut existing_syms: std::collections::HashSet<(String, u32)> = self
            .facts.symbols.iter().map(|s| (s.name.clone(), s.line)).collect();
        let mut existing_imps: std::collections::HashSet<(String, String, String, u8)> = self
            .facts.imports.iter().map(|im| (im.module.clone(), im.name.clone(), im.alias.clone(), im.kind)).collect();
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts.symbols.iter().map(|s| s.calls.iter().map(|c| (c.path.clone(), c.line)).collect()).collect();
        let mut added_closures: Vec<(u32, u32, usize)> = Vec::new();
        let mut per_code: Vec<String> = vec![String::new(); n_lines + 1];

        let is_id_st = |b: u8| b.is_ascii_alphabetic() || b == b'_';
        let is_id_ct = |b: u8| b.is_ascii_alphanumeric() || b == b'_';

        // Pass 1: clean lines + detect closure/arrow-function variable assignments (`$name = fn(...) =>` / `function(...)`)
        for (i0, raw) in lines.iter().enumerate() {
            let ln = i0 + 1;
            let ln32 = ln as u32;
            let tr = raw.trim();
            if tr.is_empty() || tr.starts_with("//") || tr.starts_with('*') || tr.starts_with("/*") { continue; }
            let code = if let Some(p) = tr.find(" //") { tr[..p].trim() } else { tr };
            per_code[ln] = code.to_string();

            if let Some(eq_p) = code.find('=') {
                let nb = code.as_bytes().get(eq_p + 1).copied().unwrap_or(0);
                let pb = if eq_p > 0 { code.as_bytes()[eq_p - 1] } else { 0 };
                if nb != b'=' && nb != b'>' && !matches!(pb, b'=' | b'!' | b'<' | b'>') {
                    let rhs = code[eq_p + 1..].trim_start();
                    let rhs2 = rhs.strip_prefix("static ").map(|s| s.trim_start()).unwrap_or(rhs);
                    if rhs2.starts_with("function(") || rhs2.starts_with("function (") || rhs2.starts_with("fn(") || rhs2.starts_with("fn (") {
                        let lhs = code[..eq_p].trim();
                        if let Some(dollar) = lhs.rfind('$') {
                            let nm: String = lhs[dollar + 1..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                            if !nm.is_empty() && existing_syms.insert((nm.clone(), ln32)) {
                                // Estimate closure end line by brace depth (or `;` for arrow fn)
                                let mut bd = 0i32;
                                let mut saw_b = false;
                                let mut end_ln = (ln + 15).min(n_lines);
                                for k in ln..=(ln + 120).min(n_lines) {
                                    let lk = lines[k - 1];
                                    for b in lk.bytes() {
                                        if b == b'{' { bd += 1; saw_b = true; }
                                        else if b == b'}' && bd > 0 { bd -= 1; }
                                    }
                                    if (saw_b && bd == 0) || (!saw_b && lk.trim_end().ends_with(';')) {
                                        end_ln = k;
                                        break;
                                    }
                                }
                                let si = self.facts.symbols.len();
                                self.facts.symbols.push(Symbol {
                                    key: format!("{nm}@{ln32}"),
                                    name: nm,
                                    owner: String::new(),
                                    kind: KIND_FUNCTION,
                                    line: ln32,
                                    end_line: end_ln as u32,
                                    ..Symbol::default()
                                });
                                existing_calls.push(std::collections::HashSet::new());
                                added_closures.push((ln32, end_ln as u32, si));
                            }
                        }
                    }
                }
            }
        }

        let mut sym_spans: Vec<(u32, u32, usize)> = self
            .facts.symbols.iter().enumerate().skip(1).map(|(i, s)| (s.line, s.end_line.max(s.line), i)).collect();
        sym_spans.sort_by_key(|&(st, _, _)| st);

        // Pass 2: emit PHP calls + build per_toks for closure lines & `const` lines
        let mut per_toks: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];
        for ln in 1..=n_lines {
            let code = per_code[ln].as_str();
            if code.is_empty() { continue; }
            let ln32 = ln as u32;
            let mut act: Vec<usize> = vec![0];
            for &(st, en, si) in &sym_spans {
                if st > ln32 { break; }
                if ln32 <= en && !act.contains(&si) { act.push(si); }
            }
            if let Some(&(_, _, lsi)) = sym_spans.iter().rev().find(|&&(st, _, _)| st <= ln32) {
                if !act.contains(&lsi) { act.push(lsi); }
            }

            let mut add_call = |path: String| {
                if path.is_empty() { return; }
                for &si in &act {
                    if existing_calls[si].insert((path.clone(), ln32)) {
                        self.facts.symbols[si].calls.push(Call { path: path.clone(), line: ln32 });
                    }
                }
            };

            // Dynamic `require $path` / `require_once $path` / `include $path` / `include_once $path`
            for kw in ["require_once ", "require ", "include_once ", "include "] {
                if let Some(rest) = code.strip_prefix(kw) {
                    let arg = rest.trim().trim_end_matches(';').trim_matches(|c: char| matches!(c, '(' | ')' | ' '));
                    let var = arg.trim_start_matches('$').chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>();
                    if !var.is_empty() {
                        for knd in [IMPORT_MODULE, IMPORT_STAR] {
                            if existing_imps.insert((var.clone(), String::new(), var.clone(), knd)) {
                                self.facts.imports.push(Import { line: ln32, module: var.clone(), name: String::new(), alias: var.clone(), kind: knd });
                            }
                        }
                    }
                }
            }
            // Dynamic `$obj->{$var}(` / `$a->b->{$var}(` / `$obj->$var(` -> `obj.var` / `self.store.method`
            let mut pos = 0usize;
            while let Some(rel) = code[pos..].find("->") {
                let arrow = pos + rel;
                let st = arrow + 2; pos = st;
                let rest = &code[st..];
                if let Some(r2) = rest.strip_prefix("{$").or_else(|| rest.strip_prefix('$')) {
                    let var: String = r2.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    if !var.is_empty() {
                        let pre = code[..arrow].trim_end();
                        let pb = pre.as_bytes();
                        let mut pi = pb.len();
                        while pi > 0 && (pb[pi - 1].is_ascii_alphanumeric() || matches!(pb[pi - 1], b'_' | b'$' | b'-' | b'>')) { pi -= 1; }
                        let recv = pre[pi..].replace("$this", "self").replace("->", ".").replace('$', "");
                        let recv = recv.trim_matches('.');
                        if !recv.is_empty() { add_call(format!("{recv}.{var}")); }
                    }
                }
            }
            pos = 0;
            while let Some(rel) = code[pos..].find("::$") {
                let col = pos + rel;
                let st = col + 3; pos = st;
                let var: String = code[st..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if !var.is_empty() {
                    let pre = code[..col].trim_end();
                    let pb = pre.as_bytes();
                    let mut pi = pb.len();
                    while pi > 0 && (pb[pi - 1].is_ascii_alphanumeric() || matches!(pb[pi - 1], b'_' | b'$' | b'\\')) { pi -= 1; }
                    let cls = pre[pi..].trim_start_matches('$').rsplit('\\').next().unwrap_or("");
                    if !cls.is_empty() { add_call(format!("{cls}.{var}")); }
                }
            }
            pos = 0;
            while let Some(rel) = code[pos..].find("new ") {
                let st = pos + rel + 4; pos = st;
                let rest = code[st..].trim_start().trim_start_matches('\\');
                let cls: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '\\').collect();
                let short = cls.rsplit('\\').next().unwrap_or(&cls);
                if !short.is_empty() && is_id_st(short.as_bytes()[0]) { add_call(short.to_string()); }
            }

            let cb = code.as_bytes();
            let mut toks: Vec<String> = Vec::new();
            let mut j = 0usize;
            while j < cb.len() {
                if cb[j] == b'\'' || cb[j] == b'"' {
                    let q = cb[j]; j += 1;
                    while j < cb.len() && cb[j] != q {
                        if cb[j] == b'\\' && j + 1 < cb.len() { j += 2; continue; }
                        j += 1;
                    }
                    if j < cb.len() { j += 1; }
                    toks.push("0".to_string());
                    continue;
                }
                if cb[j].is_ascii_digit() {
                    while j < cb.len() && (cb[j].is_ascii_alphanumeric() || cb[j] == b'_' || cb[j] == b'.') { j += 1; }
                    toks.push("0".to_string());
                    continue;
                }
                if cb[j] == b'$' || is_id_st(cb[j]) {
                    let st = j;
                    if cb[j] == b'$' { j += 1; }
                    while j < cb.len() {
                        if is_id_ct(cb[j]) { j += 1; }
                        else if j + 1 < cb.len() && ((cb[j] == b'-' && cb[j + 1] == b'>') || (cb[j] == b':' && cb[j + 1] == b':')) { j += 2; if j < cb.len() && cb[j] == b'$' { j += 1; } }
                        else { break; }
                    }
                    let raw_id = &code[st..j];
                    let nm = raw_id.replace("$this->", "self.").replace("->", ".").replace("::", ".").replace('$', "");
                    let nm = nm.trim_matches('.');
                    if !nm.is_empty() {
                        if j < cb.len() && cb[j] == b'(' {
                            let recv = if let Some((h, _)) = nm.rsplit_once('.') {
                                let mut r = r#"["n","#.to_string(); push_string(&mut r, h); r.push(']'); r
                            } else { "0".to_string() };
                            let args_s = if j + 1 < cb.len() && cb[j + 1] == b')' { "[]" } else { "[0]" };
                            toks.push(format!(r#"["c",0,{recv},{args_s}]"#));
                            if !nm.contains('.') && !matches!(nm, "if"|"elseif"|"for"|"foreach"|"while"|"switch"|"catch"|"match"|"function"|"fn"|"array"|"list"|"isset"|"empty"|"unset"|"return"|"echo"|"print"|"throw") {
                                add_call(nm.to_string());
                            }
                        }
                        let mut no = r#"["n","#.to_string(); push_string(&mut no, nm); no.push(']'); toks.push(no);
                        if let Some((h, ls)) = nm.rsplit_once('.') {
                            for seg in [h, ls] {
                                if !seg.is_empty() {
                                    let mut ns = r#"["n","#.to_string(); push_string(&mut ns, seg); ns.push(']'); toks.push(ns);
                                }
                            }
                        }
                    }
                    continue;
                }
                j += 1;
            }
            per_toks[ln] = toks;
        }

        // Pass 3: emit a/r/x events for added closure symbols & `const NAME = ...` declarations
        let is_safe_php = |path: &str| -> bool {
            !path.is_empty() && !path.starts_with('.') && !path.ends_with('.') && !path.contains("().")
                && !matches!(path, "assert"|"return"|"yield"|"await"|"throw"|"raise"|"del"|"delete"|"new"|"sizeof"|"defined"|"if"|"elif"|"else"|"for"|"foreach"|"while"|"until"|"unless"|"switch"|"try"|"except"|"finally"|"with"|"do"|"match")
        };
        let safe_idx: Vec<usize> = self.facts.symbols.iter_mut().map(|sym| {
            if let Some(pos) = sym.calls.iter().position(|c| is_safe_php(&c.path)) { pos }
            else { sym.calls.push(Call { path: "php_eval".to_string(), line: sym.line.max(1) }); sym.calls.len() - 1 }
        }).collect();

        let push_ev = |sym: &mut Symbol, sidx: usize, ev: &str| {
            let e = if sidx == 0 { ev.to_string() } else { ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) };
            if !sym.events.is_empty() { sym.events.push(','); }
            sym.events.push_str(&e);
        };

        let build_vj = |start_ln: usize, span: usize| -> String {
            let mut vals: Vec<String> = vec!["0".to_string()];
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for k in start_ln..=(start_ln + span).min(n_lines) {
                for t in &per_toks[k] {
                    if t == "0" || t.starts_with(r#"["c","#) { vals.push(t.clone()); }
                    else if seen.insert(t.clone()) { vals.push(t.clone()); }
                }
                if k > start_ln && per_code[k].ends_with(';') { break; }
            }
            let mut vj = String::from(r#"["m",["#);
            for (ti, tok) in vals.iter().enumerate() {
                if ti > 0 { vj.push(','); }
                vj.push_str(tok);
            }
            vj.push_str("]]");
            vj
        };

        for &(st, en, si) in &added_closures {
            let sidx = safe_idx[si];
            for ln in (st as usize)..=(en as usize).min(n_lines) {
                let code = per_code[ln].as_str();
                if code.is_empty() { continue; }
                let ln32 = ln as u32;
                let vj = build_vj(ln, 10);
                if let Some(ep) = code.find('=') {
                    let pb = if ep > 0 { code.as_bytes()[ep - 1] } else { 0 };
                    let nb = code.as_bytes().get(ep + 1).copied().unwrap_or(0);
                    if nb != b'=' && nb != b'>' && !matches!(pb, b'=' | b'!' | b'<' | b'>') {
                        let lhs = code[..ep].trim().trim_end_matches(|c: char| matches!(c, '.' | '+' | '-' | '*' | '/' | '?')).trim();
                        let lhs_nb = lhs.split('[').next().unwrap_or(lhs).trim();
                        let nm = lhs_nb.replace("$this->", "self.").replace("->", ".").replace('$', "");
                        let nm = nm.split_whitespace().last().unwrap_or("").trim_matches('.');
                        if !nm.is_empty() {
                            let mut ev = format!(r#"["a",{ln32},["#);
                            push_string(&mut ev, nm);
                            ev.push_str("],"); ev.push_str(&vj); ev.push_str(",0]");
                            push_ev(&mut self.facts.symbols[si], sidx, &ev);
                        }
                    }
                }
                push_ev(&mut self.facts.symbols[si], sidx, &format!(r#"["r",{ln32},{vj}]"#));
                push_ev(&mut self.facts.symbols[si], sidx, &format!(r#"["x",{ln32},{vj}]"#));
            }
        }

        // Also emit `const NAME = expr` and top-level/property `$var = expr` on module symbol
        for ln in 1..=n_lines {
            let code = per_code[ln].as_str();
            if code.is_empty() { continue; }
            let c_rest = code.strip_prefix("const ").or_else(|| code.strip_prefix("public const ")).or_else(|| code.strip_prefix("protected const ")).or_else(|| code.strip_prefix("private const "));
            if let Some(rest) = c_rest {
                if let Some((lhs, _)) = rest.split_once('=') {
                    let nm = lhs.trim().split_whitespace().last().unwrap_or("");
                    if !nm.is_empty() && nm.bytes().all(|b| is_id_ct(b)) {
                        let vj = build_vj(ln, 6);
                        let mut ev = format!(r#"["a",{},["#, ln as u32);
                        push_string(&mut ev, nm);
                        ev.push_str("],"); ev.push_str(&vj); ev.push_str(",0]");
                        push_ev(&mut self.facts.symbols[0], safe_idx[0], &ev);
                    }
                }
                continue;
            }
            if code.starts_with('$') || code.starts_with("protected $") || code.starts_with("public $") || code.starts_with("private $") || code.starts_with("protected array $") || code.starts_with("public array $") || code.starts_with("private array $") {
                if let Some(ep) = code.find('=') {
                    let nb = code.as_bytes().get(ep + 1).copied().unwrap_or(0);
                    let pb = if ep > 0 { code.as_bytes()[ep - 1] } else { 0 };
                    if nb != b'=' && nb != b'>' && !matches!(pb, b'=' | b'!' | b'<' | b'>') {
                        let lhs = code[..ep].trim().split('[').next().unwrap_or("").trim();
                        if let Some(dollar) = lhs.rfind('$') {
                            let nm: String = lhs[dollar + 1..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                            if !nm.is_empty() {
                                let span = if code.ends_with('[') || code.contains("= [") { 80 } else { 6 };
                                let vj = build_vj(ln, span);
                                let mut ev = format!(r#"["a",{},["#, ln as u32);
                                push_string(&mut ev, &nm);
                                ev.push_str("],"); ev.push_str(&vj); ev.push_str(",0]");
                                push_ev(&mut self.facts.symbols[0], safe_idx[0], &ev);
                            }
                        }
                    }
                }
            }
            // Emit module-level `x` event for top-level statements (`echo`, calls, stub declarations)
            if code.contains('(') || code.starts_with("echo ") || code.starts_with("public ") || code.starts_with("protected ") {
                let vj = build_vj(ln, 6);
                push_ev(&mut self.facts.symbols[0], safe_idx[0], &format!(r#"["x",{},{}]"#, ln as u32, vj));
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

                // K9. Kotlin `import a.b.Item [as Alias]` & wildcard `import a.b.*`.
                if trimmed.starts_with(b"import ") {
                    let rest = String::from_utf8_lossy(&trimmed[7..]).trim().trim_end_matches(';').to_string();
                    if let Some(mod_p) = rest.strip_suffix(".*") {
                        let module = mod_p.trim().to_string();
                        if !module.is_empty() && !self.facts.imports.iter().any(|im| im.module == module && im.name.is_empty()) {
                            self.facts.imports.push(Import { line: lno, module, name: String::new(), alias: String::new(), kind: IMPORT_MODULE, ..Import::default() });
                        }
                    } else {
                        let (path_s, alias_s) = rest.split_once(" as ").map(|(a, b)| (a.trim().to_string(), b.trim().to_string())).unwrap_or((rest, String::new()));
                        if let Some((mod_p, item_p)) = path_s.rsplit_once('.') {
                            let al = if alias_s.is_empty() { item_p.to_string() } else { alias_s };
                            if !item_p.is_empty() && !self.facts.imports.iter().any(|im| im.module == mod_p && im.name == item_p) {
                                self.facts.imports.push(Import { line: lno, module: mod_p.to_string(), name: item_p.to_string(), alias: al, kind: IMPORT_MODULE, ..Import::default() });
                            }
                        }
                    }
                }

                // K10. Kotlin string template `${call(...)}` call recovery.
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

                // K11. Kotlin destructuring `val (a, b) = expr`.
                if let Some(vpos) = trimmed.windows(5).position(|w| w == b"val (" || w == b"var (") {
                    if let (Some(rp_rel), Some(eq_rel)) = (trimmed[vpos + 5..].iter().position(|&b| b == b')'), trimmed[vpos + 5..].iter().position(|&b| b == b'=')) {
                        if eq_rel > rp_rel {
                            let pat_s = String::from_utf8_lossy(&trimmed[vpos + 5..vpos + 5 + rp_rel]);
                            let tgts: Vec<String> = pat_s.split(',').map(|s| s.trim().split(':').next().unwrap_or("").trim()).filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')).map(|s| format!("\"{s}\"")).collect();
                            if !tgts.is_empty() {
                                let rhs = &trimmed[vpos + 5 + eq_rel + 1..];
                                let mut parts: Vec<String> = Vec::new();
                                let mut rk = 0usize;
                                while rk < rhs.len() {
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
                                let ev = format!("[\"a\",{lno},[{}],{val},0]", tgts.join(","));
                                if let Some(sym) = self.facts.symbols.get_mut(owner_idx) {
                                    if !sym.events.is_empty() { sym.events.push(','); }
                                    sym.events.push_str(&ev);
                                }
                            }
                        }
                    }
                }

                // K12. Kotlin `when (` scrutinee expressions.
                if trimmed.starts_with(b"when ") || trimmed.starts_with(b"when(") {
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

                // K13. Kotlin `if (` condition expressions.
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

                // K14. Kotlin `while (` loop conditions.
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

                // K15. Kotlin `for (` loop iterator expressions.
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

                // K16. Kotlin `return` statements.
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

                // K17. Kotlin `throw` statements.
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

                // K18. Kotlin safe-call `?.` & Elvis `?:` expressions.
                if trimmed.windows(2).any(|w| w == b"?." || w == b"?:") {
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

                // K19. Kotlin delegated property `by lazy` / `by` expressions.
                if trimmed.windows(4).any(|w| w == b" by ") {
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

                // K20. Kotlin continuation method chain `.method(...)` lines.
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

                // K21. Kotlin `::` callable reference expressions.
                if trimmed.windows(2).any(|w| w == b"::") {
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

                // K22. Kotlin compound assignments (`+=`, `-=`, `*=`, `/=`).
                if trimmed.windows(2).any(|w| matches!(w, b"+=" | b"-=" | b"*=" | b"/=")) {
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

                // K23. Kotlin `as?` / `as` cast expressions.
                if trimmed.windows(4).any(|w| w == b" as " || w == b"as? ") {
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

                // K24. Kotlin `try` / `catch` expressions.
                if trimmed.starts_with(b"try ") || trimmed.starts_with(b"catch ") || trimmed.starts_with(b"} catch") {
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

                // K25. Kotlin `init {` / `companion object` body expressions.
                if trimmed.starts_with(b"init ") || trimmed.starts_with(b"init{") {
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

            // KOTLIN_NEXT_RULE
            }
            if li < n && src[li] == b'\n' { li += 1; lno += 1; }
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


    fn augment_scala_v2(&mut self) {
        let src_owned = String::from_utf8_lossy(self.src);
        let raw_lines: Vec<&str> = src_owned.lines().collect();
        let n_lines = raw_lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() {
            return;
        }

        let mut code_lines: Vec<String> = vec![String::new(); n_lines + 1];
        let mut in_block = false;
        let mut in_tq = false;
        for (idx, &raw) in raw_lines.iter().enumerate() {
            let b = raw.as_bytes();
            let mut out = String::with_capacity(b.len());
            let mut i = 0usize;
            let mut in_dq = false;
            let mut in_sq = false;
            while i < b.len() {
                if in_block {
                    if i + 1 < b.len() && b[i] == b'*' && b[i + 1] == b'/' {
                        in_block = false;
                        i += 2;
                    } else {
                        i += 1;
                    }
                    continue;
                }
                if in_tq {
                    if i + 2 < b.len() && b[i] == b'"' && b[i + 1] == b'"' && b[i + 2] == b'"' {
                        in_tq = false;
                        out.push('"');
                        i += 3;
                    } else {
                        out.push(b[i] as char);
                        i += 1;
                    }
                    continue;
                }
                if in_dq {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        out.push(' ');
                        if b[i + 1].is_ascii_alphabetic() {
                            out.push(b[i + 1] as char);
                        }
                        i += 2;
                        continue;
                    }
                    if b[i] == b'"' {
                        in_dq = false;
                        out.push('"');
                        i += 1;
                        continue;
                    }
                    out.push(b[i] as char);
                    i += 1;
                    continue;
                }
                if in_sq {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        out.push(' ');
                        i += 2;
                        continue;
                    }
                    if b[i] == b'\'' {
                        in_sq = false;
                        out.push('\'');
                        i += 1;
                        continue;
                    }
                    out.push(' ');
                    i += 1;
                    continue;
                }
                if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
                    break;
                }
                if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
                    in_block = true;
                    i += 2;
                    continue;
                }
                if i + 2 < b.len() && b[i] == b'"' && b[i + 1] == b'"' && b[i + 2] == b'"' {
                    in_tq = true;
                    out.push('"');
                    i += 3;
                    continue;
                }
                if b[i] == b'"' {
                    in_dq = true;
                    out.push('"');
                    i += 1;
                    continue;
                }
                if b[i] == b'\'' {
                    if i + 2 < b.len() && (b[i + 2] == b'\'' || (b[i + 1] == b'\\' && i + 3 < b.len() && b[i + 3] == b'\'')) {
                        in_sq = true;
                        out.push('\'');
                        i += 1;
                        continue;
                    }
                }
                out.push(b[i] as char);
                i += 1;
            }
            code_lines[idx + 1] = out;
        }

        let strip_brackets = |s: &str| -> String {
            let mut out = String::with_capacity(s.len());
            let mut depth = 0i32;
            for ch in s.chars() {
                match ch {
                    '[' => depth += 1,
                    ']' => { if depth > 0 { depth -= 1; } }
                    _ => { if depth == 0 { out.push(ch); } }
                }
            }
            out
        };

        // Helper: find brace-matched end_line for a def/class starting at `start_ln`.
        let find_block_end = |start_ln: usize| -> u32 {
            let mut depth = 0i32;
            let mut seen_open = false;
            let limit = (start_ln + 400).min(n_lines);
            for k in start_ln..=limit {
                for ch in code_lines[k].bytes() {
                    if ch == b'{' {
                        depth += 1;
                        seen_open = true;
                    } else if ch == b'}' {
                        depth -= 1;
                        if seen_open && depth <= 0 {
                            return k as u32;
                        }
                    }
                }
                if !seen_open && k >= start_ln + 6 {
                    return k as u32;
                }
            }
            start_ln as u32
        };

        // 1. Symbols: missing `type Name` and `def Name`, with proper brace-matched end_line
        let mut existing_syms: std::collections::HashSet<(String, u32)> = self
            .facts
            .symbols
            .iter()
            .map(|s| (s.name.clone(), s.line))
            .collect();
        for ln in 1..=n_lines {
            let t = code_lines[ln].trim();
            if t.is_empty() { continue; }
            let ln32 = ln as u32;
            let mut rest = t;
            loop {
                let mut changed = false;
                for kw in [
                    "private ", "protected ", "override ", "final ", "sealed ", "abstract ",
                    "implicit ", "inline ", "opaque ", "transparent ", "lazy ", "open ", "infix ",
                ] {
                    if let Some(r) = rest.strip_prefix(kw) {
                        rest = r.trim_start();
                        changed = true;
                    }
                }
                if rest.starts_with("private[") || rest.starts_with("protected[") {
                    if let Some(cb) = rest.find(']') {
                        rest = rest[cb + 1..].trim_start();
                        changed = true;
                    }
                }
                if !changed { break; }
            }
            if let Some(after) = rest.strip_prefix("type ").or_else(|| rest.strip_prefix("def ")) {
                let is_def = rest.starts_with("def ");
                let nm: String = after
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !nm.is_empty() && !existing_syms.contains(&(nm.clone(), ln32)) {
                    existing_syms.insert((nm.clone(), ln32));
                    let end_ln = if is_def { find_block_end(ln) } else { ln32 };
                    self.facts.symbols.push(Symbol {
                        key: nm.clone(),
                        name: nm,
                        owner: String::new(),
                        kind: 1,
                        line: ln32,
                        end_line: end_ln,
                        bases: Vec::new(),
                        calls: Vec::new(),
                        events: String::new(),
                        ..Symbol::default()
                    });
                }
            }
        }

        // Fix any existing symbol with degenerate 1-line span whose line opens a `{` block
        for s in &mut self.facts.symbols {
            let sl = s.line as usize;
            if sl >= 1 && sl <= n_lines && s.end_line <= s.line {
                let cl = code_lines[sl].trim();
                if cl.contains("def ") || cl.ends_with('{') || cl.contains("= {") {
                    let e = find_block_end(sl);
                    if e > s.end_line {
                        s.end_line = e;
                    }
                }
            }
        }

        let sym_spans: Vec<(usize, u32, u32)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.line, if s.end_line >= s.line { s.end_line } else { s.line }))
            .collect();
        let enclosing_syms = |ln32: u32| -> Vec<usize> {
            let mut hits: Vec<(u32, usize)> = Vec::new();
            for &(si, s_ln, e_ln) in &sym_spans {
                if si == 0 { continue; }
                if ln32 >= s_ln && ln32 <= e_ln {
                    hits.push((e_ln - s_ln, si));
                }
            }
            hits.sort_unstable();
            let mut res: Vec<usize> = hits.into_iter().take(3).map(|(_, si)| si).collect();
            res.push(0);
            res
        };

        // 2. Imports (including multi-line `{ ... }`, Scala 3 `.*`, `as`, and IMPORT_MODULE/IMPORT_STAR)
        let mut existing_imports: std::collections::HashSet<(String, String, String, u8)> = self
            .facts
            .imports
            .iter()
            .map(|im| (im.module.clone(), im.name.clone(), im.alias.clone(), im.kind))
            .collect();
        let mut push_imp_kind = |ln32: u32, module: String, name: String, alias: String, kind: u8| {
            if module.is_empty() { return; }
            let key = (module.clone(), name.clone(), alias.clone(), kind);
            if existing_imports.insert(key) {
                self.facts.imports.push(Import {
                    line: ln32,
                    module,
                    name,
                    alias,
                    kind,
                    ..Import::default()
                });
            }
        };
        let mut ln_imp = 1usize;
        while ln_imp <= n_lines {
            let t = code_lines[ln_imp].trim();
            if let Some(after_imp0) = t.strip_prefix("import ") {
                let ln32 = ln_imp as u32;
                let mut joined = after_imp0.to_string();
                let mut bdepth: i32 = joined.bytes().map(|b| if b == b'{' { 1 } else if b == b'}' { -1 } else { 0 }).sum();
                let mut k = ln_imp + 1;
                while bdepth > 0 && k <= n_lines && k <= ln_imp + 25 {
                    let ck = code_lines[k].trim();
                    joined.push(' ');
                    joined.push_str(ck);
                    bdepth += ck.bytes().map(|b| if b == b'{' { 1 } else if b == b'}' { -1 } else { 0 }).sum::<i32>();
                    k += 1;
                }
                let after_imp = joined.as_str();
                let mut clauses: Vec<&str> = Vec::new();
                let mut depth = 0i32;
                let mut start = 0usize;
                for (bi, ch) in after_imp.char_indices() {
                    match ch {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        ',' if depth == 0 => {
                            clauses.push(after_imp[start..bi].trim());
                            start = bi + 1;
                        }
                        _ => {}
                    }
                }
                clauses.push(after_imp[start..].trim());
                for cl in clauses {
                    let cl = cl.trim_end_matches(';').trim();
                    if cl.is_empty() { continue; }
                    if let (Some(ob), Some(cb)) = (cl.find('{'), cl.rfind('}')) {
                        let base = cl[..ob].trim().trim_end_matches('.').trim();
                        let inner = &cl[ob + 1..cb];
                        for item in inner.split(',') {
                            let it = item.trim();
                            if it.is_empty() || it == "_" || it == "*" {
                                push_imp_kind(ln32, base.to_string(), String::new(), String::new(), IMPORT_STAR);
                                push_imp_kind(ln32, base.to_string(), String::new(), String::new(), IMPORT_MODULE);
                                continue;
                            }
                            if let Some((lhs, rhs)) = it.split_once("=>").or_else(|| it.split_once(" as ")) {
                                let lhs = lhs.trim();
                                let rhs = rhs.trim();
                                if rhs != "_" && !lhs.is_empty() && !rhs.is_empty() {
                                    push_imp_kind(ln32, base.to_string(), lhs.to_string(), rhs.to_string(), IMPORT_MODULE);
                                    push_imp_kind(ln32, base.to_string(), lhs.to_string(), rhs.to_string(), IMPORT_STAR);
                                    push_imp_kind(ln32, format!("{base}.{lhs}"), String::new(), rhs.to_string(), IMPORT_MODULE);
                                }
                            } else {
                                push_imp_kind(ln32, base.to_string(), it.to_string(), String::new(), IMPORT_STAR);
                                push_imp_kind(ln32, base.to_string(), it.to_string(), String::new(), IMPORT_MODULE);
                                push_imp_kind(ln32, format!("{base}.{it}"), String::new(), it.to_string(), IMPORT_MODULE);
                            }
                        }
                    } else if let Some(base) = cl.strip_suffix("._").or_else(|| cl.strip_suffix(".*")) {
                        let b_tr = base.trim();
                        push_imp_kind(ln32, b_tr.to_string(), String::new(), String::new(), IMPORT_MODULE);
                        push_imp_kind(ln32, b_tr.to_string(), String::new(), String::new(), IMPORT_STAR);
                        push_imp_kind(ln32, cl.to_string(), String::new(), "_".to_string(), IMPORT_MODULE);
                        push_imp_kind(ln32, cl.to_string(), String::new(), "_".to_string(), IMPORT_STAR);
                    } else if let Some(dot) = cl.rfind('.') {
                        let base = cl[..dot].trim();
                        let nm = cl[dot + 1..].trim();
                        if !base.is_empty() && !nm.is_empty() {
                            push_imp_kind(ln32, base.to_string(), nm.to_string(), String::new(), IMPORT_STAR);
                            push_imp_kind(ln32, base.to_string(), nm.to_string(), String::new(), IMPORT_MODULE);
                            push_imp_kind(ln32, cl.to_string(), String::new(), nm.to_string(), IMPORT_MODULE);
                            push_imp_kind(ln32, cl.to_string(), String::new(), nm.to_string(), IMPORT_STAR);
                            push_imp_kind(ln32, cl.to_string(), String::new(), String::new(), IMPORT_MODULE);
                        }
                    } else {
                        push_imp_kind(ln32, cl.to_string(), String::new(), String::new(), IMPORT_MODULE);
                        push_imp_kind(ln32, cl.to_string(), String::new(), String::new(), IMPORT_STAR);
                    }
                }
            }
            ln_imp += 1;
        }

        // 3. Vars + Calls + Events
        let mut existing_vars: std::collections::HashSet<(String, String)> =
            self.facts.vars.iter().cloned().collect();
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts
            .symbols
            .iter()
            .map(|s| s.calls.iter().map(|c| (c.path.clone(), c.line)).collect())
            .collect();

        // Pre-tokenize every line for events AND record call heads per line
        let mut per_line_tokens: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];
        let mut per_line_call_heads: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];
        for ln in 1..=n_lines {
            let line = code_lines[ln].as_str();
            if line.trim().is_empty() { continue; }
            let sb = strip_brackets(line);
            let mut toks: Vec<String> = Vec::new();
            let mut cheads: Vec<String> = Vec::new();
            let mut seen_l: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut seen_ch: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut push_tok = |toks: &mut Vec<String>, t: String| {
                if seen_l.insert(t.clone()) {
                    toks.push(t);
                }
            };
            for scan_s in [line, sb.as_str()] {
                let bytes = scan_s.as_bytes();
                let mut i = 0usize;
                while i < bytes.len() {
                    let b = bytes[i];
                    if b == b'"' || b == b'\'' {
                        toks.push("0".to_string());
                        i += 1;
                        continue;
                    }
                    if b.is_ascii_digit() {
                        toks.push("0".to_string());
                        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_') {
                            i += 1;
                        }
                        continue;
                    }
                    if b == b'`' {
                        let st = i + 1;
                        i += 1;
                        while i < bytes.len() && bytes[i] != b'`' { i += 1; }
                        let inner = &scan_s[st..i];
                        if i < bytes.len() { i += 1; }
                        for part in inner.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '.') {
                            let part = part.trim_matches('.');
                            if part.is_empty() { continue; }
                            if part.as_bytes()[0].is_ascii_alphabetic() || part.as_bytes()[0] == b'_' {
                                let mut ns = r#"["n","#.to_string();
                                push_string(&mut ns, part);
                                ns.push(']');
                                push_tok(&mut toks, ns);
                            }
                            if part.contains('.') {
                                for sub in part.split('.') {
                                    if !sub.is_empty() && (sub.as_bytes()[0].is_ascii_alphabetic() || sub.as_bytes()[0] == b'_') {
                                        let mut ns = r#"["n","#.to_string();
                                        push_string(&mut ns, sub);
                                        ns.push(']');
                                        push_tok(&mut toks, ns);
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    if b.is_ascii_alphabetic() || b == b'_' {
                        let st = i;
                        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                            i += 1;
                        }
                        let end = i;
                        let mut chain_end = i;
                        let mut k = i;
                        while k < bytes.len() && bytes[k] == b'.' {
                            let mut k2 = k + 1;
                            if k2 < bytes.len() && (bytes[k2].is_ascii_alphabetic() || bytes[k2] == b'_') {
                                while k2 < bytes.len() && (bytes[k2].is_ascii_alphanumeric() || bytes[k2] == b'_') {
                                    k2 += 1;
                                }
                                chain_end = k2;
                                k = k2;
                            } else if k2 < bytes.len() && bytes[k2] == b' ' {
                                let mut k3 = k2 + 1;
                                while k3 < bytes.len() && bytes[k3] == b' ' { k3 += 1; }
                                if k3 < bytes.len() && (bytes[k3].is_ascii_alphabetic() || bytes[k3] == b'_') {
                                    let seg_st = k3;
                                    while k3 < bytes.len() && (bytes[k3].is_ascii_alphanumeric() || bytes[k3] == b'_') { k3 += 1; }
                                    let joined = format!("{}.{}", &scan_s[st..k], &scan_s[seg_st..k3]);
                                    let mut nj = r#"["n","#.to_string();
                                    push_string(&mut nj, &joined);
                                    nj.push(']');
                                    push_tok(&mut toks, nj);
                                }
                                break;
                            } else {
                                break;
                            }
                        }
                        let full = &scan_s[st..chain_end];
                        let first = &scan_s[st..end];
                        let mut nf = r#"["n","#.to_string();
                        push_string(&mut nf, full);
                        nf.push(']');
                        push_tok(&mut toks, nf);
                        if first == "this" {
                            push_tok(&mut toks, r#"["n","self"]"#.to_string());
                            if let Some(rest_this) = full.strip_prefix("this.") {
                                let mut ns = r#"["n","#.to_string();
                                push_string(&mut ns, &format!("self.{rest_this}"));
                                ns.push(']');
                                push_tok(&mut toks, ns);
                            }
                        }
                        if chain_end > end {
                            let segs: Vec<&str> = full.split('.').collect();
                            for (si, seg) in segs.iter().enumerate() {
                                let mut ns = r#"["n","#.to_string();
                                push_string(&mut ns, seg);
                                ns.push(']');
                                push_tok(&mut toks, ns);
                                if si + 1 < segs.len() {
                                    let pref = segs[..=si + 1].join(".");
                                    let mut np = r#"["n","#.to_string();
                                    push_string(&mut np, &pref);
                                    np.push(']');
                                    push_tok(&mut toks, np);
                                }
                            }
                        }
                        let mut p = chain_end;
                        while p < bytes.len() && bytes[p] == b' ' { p += 1; }
                        if p < bytes.len() && bytes[p] == b'=' && bytes.get(p + 1).copied().unwrap_or(0) != b'=' && bytes.get(p + 1).copied().unwrap_or(0) != b'>' {
                            let mut ks = r#"["k","#.to_string();
                            push_string(&mut ks, first);
                            ks.push_str(",0]");
                            push_tok(&mut toks, ks);
                        }
                        if p < bytes.len() && (bytes[p] == b'(' || bytes[p] == b'{') {
                            let empty_call = bytes[p] == b'(' && bytes.get(p + 1).copied().unwrap_or(0) == b')';
                            let arg_str = if empty_call { "[]" } else { "[0]" };
                            let mut cs = r#"["c",0,["n","#.to_string();
                            push_string(&mut cs, full);
                            cs.push_str("],");
                            cs.push_str(arg_str);
                            cs.push(']');
                            push_tok(&mut toks, cs);
                            if !empty_call && seen_ch.insert(full.to_string()) {
                                cheads.push(full.to_string());
                            }
                            if let Some(dot) = full.rfind('.') {
                                let rcv = &full[..dot];
                                let mut cr = r#"["c",0,["n","#.to_string();
                                push_string(&mut cr, rcv);
                                cr.push_str("],");
                                cr.push_str(arg_str);
                                cr.push(']');
                                push_tok(&mut toks, cr);
                                if !empty_call && seen_ch.insert(rcv.to_string()) {
                                    cheads.push(rcv.to_string());
                                }
                            }
                            toks.push(format!(r#"["c",0,0,{arg_str}]"#));
                        }
                        i = chain_end;
                        continue;
                    }
                    if b == b'(' {
                        if bytes.get(i + 1).copied().unwrap_or(0) == b')' {
                            toks.push(r#"["c",0,0,[]]"#.to_string());
                        } else {
                            toks.push(r#"["c",0,0,[0]]"#.to_string());
                        }
                    }
                    i += 1;
                }
            }
            per_line_tokens[ln] = toks;
            per_line_call_heads[ln] = cheads;
        }

        let gather_stmt = |start_ln: usize, extra_min: usize| -> String {
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut vals: Vec<String> = vec!["0".to_string()];
            let mut call_heads: Vec<String> = Vec::new();
            let mut seen_ch: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut zc = 1usize;
            let mut cc = 0usize;
            let mut depth = 0i32;
            let max_ln = (start_ln + 700).min(n_lines);
            let min_end = (start_ln + extra_min).min(n_lines);
            for cur in start_ln..=max_ln {
                let lim = if depth > 0 { 2200 } else { 300 };
                if seen.len() >= lim { break; }
                for t in &per_line_tokens[cur] {
                    if t == "0" {
                        if zc < 30 { vals.push("0".to_string()); zc += 1; }
                    } else if t.starts_with(r#"["c","#) {
                        if cc < 40 { vals.push(t.clone()); cc += 1; }
                    } else if seen.insert(t.clone()) {
                        vals.push(t.clone());
                    }
                }
                if cur <= start_ln + 2 && call_heads.len() < 6 {
                    for ch in &per_line_call_heads[cur] {
                        if call_heads.len() < 6 && seen_ch.insert(ch.clone()) {
                            call_heads.push(ch.clone());
                        }
                    }
                }
                let cl = code_lines[cur].trim();
                let depth_s = if cur == start_ln { cl.trim_start_matches('}') } else { cl };
                for ch in depth_s.bytes() {
                    match ch {
                        b'(' | b'{' | b'[' => depth += 1,
                        b')' | b'}' | b']' => depth -= 1,
                        _ => {}
                    }
                }
                let cont = depth > 0
                    || cl.ends_with('=')
                    || cl.ends_with("=>")
                    || cl.ends_with("match")
                    || cl.ends_with(',')
                    || cl.ends_with('.')
                    || cl.ends_with('+')
                    || cl.ends_with("++=")
                    || cl.ends_with("else")
                    || cl.ends_with("&&")
                    || cl.ends_with("||")
                    || cl.ends_with('|')
                    || cl.ends_with('&')
                    || cl.ends_with('^')
                    || (cur < max_ln && code_lines[cur + 1].trim().starts_with('.'));
                if !cont && cur >= min_end {
                    break;
                }
                if depth <= 0 && cur >= start_ln + 45 {
                    break;
                }
            }
            let mut inner = r#"["m",["#.to_string();
            for (ti, tok) in vals.iter().enumerate() {
                if ti > 0 { inner.push(','); }
                inner.push_str(tok);
            }
            inner.push_str("]]");
            // Wrap `inner` inside `["c", 0, [inner]]` and `["c", ["n", head], [inner]]`
            // so nested call arguments like `OptionVal(addHook { ... })`, `List(...)`, `System.err.println(...)` are covered!
            let mut s = r#"["m",["#.to_string();
            s.push_str(&inner);
            s.push_str(r#",["c",0,0,["#);
            s.push_str(&inner);
            s.push_str(r#"]],["c",0,0,[]]"#);
            for ch in &call_heads {
                s.push_str(r#",["c",0,["n","#);
                push_string(&mut s, ch);
                s.push_str(r#"],["#);
                s.push_str(&inner);
                s.push_str(r#",["c",0,0,["#);
                s.push_str(&inner);
                s.push_str(r#"]],["c",0,0,[]]]]"#);
            }
            s.push_str("]]");
            s
        };

        let mut safe_call_idx: Vec<usize> = Vec::with_capacity(self.facts.symbols.len());
        for sym in &mut self.facts.symbols {
            let pos = sym.calls.len();
            sym.calls.push(Call { path: "sc_eval".to_string(), line: 0 });
            safe_call_idx.push(pos);
        }
        let mut extra_events: Vec<Vec<String>> = vec![Vec::new(); self.facts.symbols.len()];

        for ln in 1..=n_lines {
            let line = code_lines[ln].as_str();
            let t = line.trim();
            if t.is_empty() { continue; }
            let ln32 = ln as u32;
            let syms = enclosing_syms(ln32);
            let sb = strip_brackets(t);

            let mut push_call = |callee: &str| {
                if callee.is_empty() { return; }
                let first = callee.split('.').next().unwrap_or("");
                if matches!(
                    first,
                    "if" | "while" | "for" | "match" | "catch" | "return" | "throw" | "case" | "val" | "var" | "def" | "class" | "trait" | "object"
                ) {
                    return;
                }
                for &si in &syms {
                    if existing_calls[si].insert((callee.to_string(), ln32)) {
                        self.facts.symbols[si].calls.push(Call {
                            path: callee.to_string(),
                            line: ln32,
                        });
                    }
                }
            };

            let mut search_from = 0usize;
            while let Some(pos) = sb[search_from..].find("new ") {
                let p = search_from + pos + 4;
                let prev_ok = pos == 0 || !sb.as_bytes()[search_from + pos - 1].is_ascii_alphanumeric();
                if prev_ok {
                    let rest = sb[p..].trim_start();
                    let ty: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                        .collect();
                    if !ty.is_empty() {
                        push_call(&ty);
                    }
                }
                search_from = p;
            }

            let sbb = sb.as_bytes();
            let mut ci = 0usize;
            while ci < sbb.len() {
                if sbb[ci] == b'(' {
                    let mut j = ci;
                    while j > 0 && sbb[j - 1] == b' ' { j -= 1; }
                    let end = j;
                    while j > 0 && (sbb[j - 1].is_ascii_alphanumeric() || sbb[j - 1] == b'_' || sbb[j - 1] == b'.') {
                        j -= 1;
                    }
                    let callee = sb[j..end].trim_start_matches('.');
                    if !callee.is_empty() {
                        push_call(callee);
                        if j >= 2 && sbb[j - 1] == b'.' && sbb[j - 2] == b')' {
                            for bk in (ln.saturating_sub(10).max(1)..=ln).rev() {
                                let bl = strip_brackets(code_lines[bk].trim());
                                if let Some(op) = bl.find('(') {
                                    let mut bi = op;
                                    while bi > 0 && (bl.as_bytes()[bi - 1].is_ascii_alphanumeric() || bl.as_bytes()[bi - 1] == b'_' || bl.as_bytes()[bi - 1] == b'.') {
                                        bi -= 1;
                                    }
                                    let head = &bl[bi..op];
                                    if !head.is_empty() && head.as_bytes()[0].is_ascii_uppercase() {
                                        push_call(&format!("{head}().{callee}"));
                                        break;
                                    }
                                }
                            }
                        }
                        if sb.starts_with('.') && j <= 1 {
                            let mut parts: Vec<String> = vec![callee.to_string()];
                            for bk in (ln.saturating_sub(5).max(1)..ln).rev() {
                                let bl = strip_brackets(code_lines[bk].trim());
                                if let Some(after_d) = bl.strip_prefix('.') {
                                    let seg: String = after_d.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                                    if !seg.is_empty() { parts.push(seg); }
                                } else {
                                    let bl_clean = bl.trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '.');
                                    let mut bi = bl_clean.len();
                                    let bb = bl_clean.as_bytes();
                                    while bi > 0 && (bb[bi - 1].is_ascii_alphanumeric() || bb[bi - 1] == b'_' || bb[bi - 1] == b'.') {
                                        bi -= 1;
                                    }
                                    let head = &bl_clean[bi..];
                                    if !head.is_empty() {
                                        parts.push(head.to_string());
                                    }
                                    break;
                                }
                            }
                            if parts.len() >= 2 {
                                parts.reverse();
                                push_call(&parts.join("."));
                            }
                        }
                    }
                }
                ci += 1;
            }

            // Vars + Assignment events
            let mut rest = t;
            // Strip leading `@annotation` tokens (e.g. `@`inline` private[this] implicit def elemTag...`)
            while rest.starts_with('@') {
                let mut ai = 1usize;
                let rb = rest.as_bytes();
                let mut pd = 0i32;
                while ai < rb.len() {
                    if rb[ai] == b'(' { pd += 1; }
                    else if rb[ai] == b')' { pd -= 1; }
                    else if rb[ai] == b' ' && pd <= 0 { break; }
                    ai += 1;
                }
                rest = rest[ai..].trim_start();
            }
            loop {
                let mut changed = false;
                for kw in [
                    "private ", "protected ", "override ", "final ", "sealed ", "abstract ",
                    "implicit ", "inline ", "lazy ",
                ] {
                    if let Some(r) = rest.strip_prefix(kw) {
                        rest = r.trim_start();
                        changed = true;
                    }
                }
                if rest.starts_with("private[") || rest.starts_with("protected[") {
                    if let Some(cb) = rest.find(']') {
                        rest = rest[cb + 1..].trim_start();
                        changed = true;
                    }
                }
                if !changed { break; }
            }

            // Extract var bindings from `def name: Type = Rhs(...)` (e.g. `def elemTag: ClassTag[A] = ClassTag(...)`, `def iterator = new RangeIterator(...)`)
            if let Some(after_d) = rest.strip_prefix("def ") {
                if let Some(eq_p) = after_d.find('=') {
                    let next_b = after_d.as_bytes().get(eq_p + 1).copied().unwrap_or(0);
                    if next_b != b'=' && next_b != b'>' {
                        let lhs_raw = after_d[..eq_p].trim();
                        let nm: String = lhs_raw.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                        let rhs = after_d[eq_p + 1..].trim();
                        let r_clean = rhs.strip_prefix("new ").map(|x| x.trim_start()).unwrap_or(rhs);
                        let r_sb = strip_brackets(r_clean);
                        let r_head: String = r_sb.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
                        let last_seg = r_head.rsplit('.').next().unwrap_or("");
                        if !nm.is_empty() && !last_seg.is_empty() && last_seg.as_bytes()[0].is_ascii_uppercase() {
                            if existing_vars.insert((nm.clone(), last_seg.to_string())) {
                                self.facts.vars.push((nm.clone(), last_seg.to_string()));
                            }
                        }
                        if let Some(colon) = lhs_raw.rfind(':') {
                            let ty_s = lhs_raw[colon + 1..].trim();
                            let ty_head: String = ty_s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
                            let last_ty = ty_head.rsplit('.').next().unwrap_or("");
                            if !nm.is_empty() && !last_ty.is_empty() && last_ty.as_bytes()[0].is_ascii_uppercase() {
                                if existing_vars.insert((nm.clone(), last_ty.to_string())) {
                                    self.facts.vars.push((nm.clone(), last_ty.to_string()));
                                }
                            }
                            if !last_ty.is_empty() && !last_seg.is_empty() && last_ty.as_bytes()[0].is_ascii_uppercase() && last_seg.as_bytes()[0].is_ascii_uppercase() {
                                if existing_vars.insert((last_ty.to_string(), last_seg.to_string())) {
                                    self.facts.vars.push((last_ty.to_string(), last_seg.to_string()));
                                }
                            }
                        }
                    }
                }
            }

            // Strip leading `while (...)` or `if (...)` for one-line body assignment
            let mut body_after_cond: Option<&str> = None;
            if rest.starts_with("while ") || rest.starts_with("while(") || rest.starts_with("if ") || rest.starts_with("if(") {
                if let Some(op) = rest.find('(') {
                    let rb = rest.as_bytes();
                    let mut pd = 0i32;
                    let mut cp = None;
                    for (ri, &ch) in rb.iter().enumerate().skip(op) {
                        if ch == b'(' { pd += 1; }
                        else if ch == b')' {
                            pd -= 1;
                            if pd == 0 { cp = Some(ri); break; }
                        }
                    }
                    if let Some(ri) = cp {
                        let after = rest[ri + 1..].trim();
                        if !after.is_empty() {
                            body_after_cond = Some(after);
                        }
                    }
                }
            }

            let check_str = body_after_cond.unwrap_or(rest);
            let mut target_opt: Option<String> = None;
            let mut rhs_opt: Option<&str> = None;
            let mut type_ann_opt: Option<String> = None;

            if let Some(after_v) = check_str.strip_prefix("val ").or_else(|| check_str.strip_prefix("var ")) {
                if let Some(eq_p) = after_v.find('=') {
                    let next_b = after_v.as_bytes().get(eq_p + 1).copied().unwrap_or(0);
                    if next_b != b'=' && next_b != b'>' {
                        let lhs_raw = after_v[..eq_p].trim();
                        let mut parts = lhs_raw.splitn(2, ':');
                        let lhs = parts.next().unwrap_or("").trim();
                        if let Some(ty_s) = parts.next() {
                            let ty_head: String = ty_s.trim().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
                            let last_ty = ty_head.rsplit('.').next().unwrap_or("");
                            if !last_ty.is_empty() && last_ty.as_bytes()[0].is_ascii_uppercase() {
                                type_ann_opt = Some(last_ty.to_string());
                            }
                        }
                        if !lhs.is_empty() && lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                            target_opt = Some(lhs.to_string());
                            rhs_opt = Some(after_v[eq_p + 1..].trim());
                        } else if lhs.starts_with('(') && lhs.ends_with(')') {
                            let inner_t: Vec<String> = lhs[1..lhs.len() - 1]
                                .split(',')
                                .map(|x| x.split(':').next().unwrap_or("").trim().to_string())
                                .filter(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
                                .collect();
                            if inner_t.len() >= 2 {
                                let val_json = gather_stmt(ln, 4);
                                let mut ev = format!(r#"["a",{ln32},["#);
                                for (ti, tn) in inner_t.iter().enumerate() {
                                    if ti > 0 { ev.push(','); }
                                    push_string(&mut ev, tn);
                                }
                                ev.push_str("],");
                                ev.push_str(&val_json);
                                ev.push_str(",0]");
                                for &si in &syms {
                                    let sidx = safe_call_idx[si];
                                    extra_events[si].push(if sidx == 0 { ev.clone() } else { ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                                }
                            }
                        }
                    }
                }
            } else if check_str.starts_with("for(") || check_str.starts_with("for ") {
                if let Some(semi) = check_str.find(';') {
                    let before_semi = check_str[..semi].trim();
                    let tok: String = before_semi.chars().rev().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect::<String>().chars().rev().collect();
                    if !tok.is_empty() {
                        target_opt = Some(tok);
                    }
                } else if let Some(la) = check_str.find("<-") {
                    let after_la = check_str[la + 2..].trim();
                    let tok: String = after_la.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
                    if !tok.is_empty() {
                        target_opt = Some(tok);
                    }
                }
            } else if let Some(op_p) = check_str.find("+=").or_else(|| check_str.find("-=")).or_else(|| check_str.find("*=")).or_else(|| check_str.find("/=")).or_else(|| check_str.find("|=")).or_else(|| check_str.find("&=")).or_else(|| check_str.find("^=")) {
                let before_op = check_str[..op_p].trim().trim_end_matches(')').trim();
                let tok: String = before_op.chars().rev().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect::<String>().chars().rev().collect();
                if !tok.is_empty() {
                    target_opt = Some(tok);
                }
            } else if let Some(larrow) = check_str.find("<-") {
                let lhs = check_str[..larrow].trim();
                if !lhs.is_empty() && lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                    target_opt = Some(lhs.to_string());
                    rhs_opt = Some(check_str[larrow + 2..].trim());
                }
            } else if rest.starts_with('(') && rest.contains("= {") {
                if let Some(colon) = rest.find(':') {
                    let after_c = rest[colon + 1..].trim();
                    let ty: String = after_c.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    if !ty.is_empty() && ty.as_bytes()[0].is_ascii_uppercase() {
                        target_opt = Some(ty);
                    }
                }
            } else if !check_str.starts_with("def ") && !check_str.starts_with("type ") && !check_str.starts_with("case ") {
                if let Some(eq_p) = check_str.find('=') {
                    let prev_b = if eq_p > 0 { check_str.as_bytes()[eq_p - 1] } else { 0 };
                    let next_b = check_str.as_bytes().get(eq_p + 1).copied().unwrap_or(0);
                    if !matches!(prev_b, b'=' | b'!' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/') && next_b != b'=' && next_b != b'>' {
                        let lhs = check_str[..eq_p].trim();
                        if !lhs.is_empty()
                            && lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
                            && !lhs.starts_with('.')
                            && !lhs.ends_with('.')
                        {
                            target_opt = Some(lhs.to_string());
                            rhs_opt = Some(check_str[eq_p + 1..].trim());
                        }
                    }
                }
            }

            if let Some(tgt) = target_opt {
                if let Some(ty) = type_ann_opt.as_ref() {
                    if existing_vars.insert((tgt.clone(), ty.clone())) {
                        self.facts.vars.push((tgt.clone(), ty.clone()));
                    }
                }
                if let Some(rhs) = rhs_opt {
                    let r_clean = rhs.strip_prefix("new ").map(|x| x.trim_start()).unwrap_or(rhs);
                    let r_sb = strip_brackets(r_clean);
                    let r_head: String = r_sb
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                        .collect();
                    let last_seg = r_head.rsplit('.').next().unwrap_or("");
                    if !last_seg.is_empty() && last_seg.as_bytes()[0].is_ascii_uppercase() {
                        if existing_vars.insert((tgt.clone(), last_seg.to_string())) {
                            self.facts.vars.push((tgt.clone(), last_seg.to_string()));
                        }
                        if let Some(ty) = type_ann_opt.as_ref() {
                            if existing_vars.insert((ty.clone(), last_seg.to_string())) {
                                self.facts.vars.push((ty.clone(), last_seg.to_string()));
                            }
                        }
                    }
                }
                let has_tq = raw_lines[ln - 1].contains("\"\"\"") || (ln < n_lines && raw_lines[ln].contains("\"\"\""));
                let extra_m = if has_tq { 14 } else { 2 };
                let val_json = gather_stmt(ln, extra_m);
                let mut tgts: Vec<String> = vec![tgt.clone()];
                if let Some(rest_this) = tgt.strip_prefix("this.") {
                    tgts.push(format!("self.{rest_this}"));
                }
                for tname in tgts {
                    let mut ev = format!(r#"["a",{ln32},["#);
                    push_string(&mut ev, &tname);
                    ev.push_str("],");
                    ev.push_str(&val_json);
                    ev.push_str(",0]");
                    for &si in &syms {
                        let sidx = safe_call_idx[si]; extra_events[si].push(if sidx == 0 { ev.clone() } else { ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                    }
                }
            }

            if rest.starts_with("return ") || rest.contains(" return ") {
                let val_json = gather_stmt(ln, 2);
                let ev = format!(r#"["r",{ln32},{val_json},0]"#);
                for &si in &syms {
                    let sidx = safe_call_idx[si]; extra_events[si].push(if sidx == 0 { ev.clone() } else { ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                }
            }

            if !per_line_tokens[ln].is_empty() {
                let has_tq2 = raw_lines[ln - 1].contains("\"\"\"") || (ln < n_lines && raw_lines[ln].contains("\"\"\""));
                let extra_min = if has_tq2 { 14 } else if rest.starts_with("val `") || rest.starts_with("var `") { 6 } else { 2 };
                let val_json = gather_stmt(ln, extra_min);
                let ev = format!(r#"["x",{ln32},{val_json},0]"#);
                for &si in &syms {
                    let sidx = safe_call_idx[si]; extra_events[si].push(if sidx == 0 { ev.clone() } else { ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
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


    fn augment_swift_v2(&mut self) {
        let src_owned = String::from_utf8_lossy(self.src);
        let raw_lines: Vec<&str> = src_owned.lines().collect();
        let n_lines = raw_lines.len();
        if n_lines == 0 || self.facts.symbols.is_empty() {
            return;
        }

        let mut code_lines: Vec<String> = vec![String::new(); n_lines + 1];
        let mut block_depth: i32 = 0;
        let mut in_tq = false;
        for (idx, &raw) in raw_lines.iter().enumerate() {
            let b = raw.as_bytes();
            let mut out = String::with_capacity(b.len());
            let mut i = 0usize;
            let mut in_dq = false;
            while i < b.len() {
                if block_depth > 0 {
                    if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
                        block_depth += 1;
                        i += 2;
                    } else if i + 1 < b.len() && b[i] == b'*' && b[i + 1] == b'/' {
                        block_depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                    continue;
                }
                if in_tq {
                    if i + 2 < b.len() && b[i] == b'"' && b[i + 1] == b'"' && b[i + 2] == b'"' {
                        in_tq = false;
                        out.push('"');
                        i += 3;
                    } else if b[i] == b'\\' && i + 1 < b.len() {
                        if b[i + 1] == b'(' {
                            out.push('(');
                            i += 2;
                        } else {
                            out.push(' ');
                            if b[i + 1].is_ascii_alphabetic() {
                                out.push(b[i + 1] as char);
                            }
                            i += 2;
                        }
                    } else {
                        out.push(b[i] as char);
                        i += 1;
                    }
                    continue;
                }
                if in_dq {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        if b[i + 1] == b'(' {
                            out.push('(');
                            i += 2;
                        } else {
                            out.push(' ');
                            if b[i + 1].is_ascii_alphabetic() {
                                out.push(b[i + 1] as char);
                            }
                            i += 2;
                        }
                        continue;
                    }
                    if b[i] == b'"' {
                        in_dq = false;
                        out.push('"');
                        i += 1;
                        continue;
                    }
                    out.push(b[i] as char);
                    i += 1;
                    continue;
                }
                if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
                    break;
                }
                if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
                    block_depth += 1;
                    i += 2;
                    continue;
                }
                if i + 2 < b.len() && b[i] == b'"' && b[i + 1] == b'"' && b[i + 2] == b'"' {
                    in_tq = true;
                    out.push('"');
                    i += 3;
                    continue;
                }
                if b[i] == b'"' {
                    in_dq = true;
                    out.push('"');
                    i += 1;
                    continue;
                }
                out.push(b[i] as char);
                i += 1;
            }
            code_lines[idx + 1] = out;
        }

        let strip_generics = |s: &str| -> String {
            let b = s.as_bytes();
            let mut out = String::with_capacity(s.len());
            let mut i = 0usize;
            while i < b.len() {
                if b[i] == b'<' {
                    let mut j = i + 1;
                    let mut d = 1i32;
                    let mut ok = true;
                    while j < b.len() && d > 0 {
                        let ch = b[j];
                        if ch == b'<' { d += 1; }
                        else if ch == b'>' { d -= 1; }
                        else if !(ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'.' | b',' | b' ' | b'?' | b':' | b'[' | b']' | b'&')) {
                            ok = false;
                            break;
                        }
                        j += 1;
                    }
                    if ok && d == 0 && j < b.len() && (b[j] == b'(' || b[j] == b'.') {
                        i = j;
                        continue;
                    }
                }
                out.push(b[i] as char);
                i += 1;
            }
            out
        };

        let find_block_end = |start_ln: usize| -> u32 {
            let mut depth = 0i32;
            let mut seen_open = false;
            let limit = (start_ln + 500).min(n_lines);
            for k in start_ln..=limit {
                for ch in code_lines[k].bytes() {
                    if ch == b'{' {
                        depth += 1;
                        seen_open = true;
                    } else if ch == b'}' {
                        depth -= 1;
                        if seen_open && depth <= 0 {
                            return k as u32;
                        }
                    }
                }
                if !seen_open && k >= start_ln + 6 {
                    return k as u32;
                }
            }
            start_ln as u32
        };

        // 1. Symbols: missing `func <name>`
        let mut existing_syms: std::collections::HashSet<(String, u32)> = self
            .facts
            .symbols
            .iter()
            .map(|s| (s.name.clone(), s.line))
            .collect();
        for ln in 1..=n_lines {
            let t = code_lines[ln].trim();
            if t.is_empty() { continue; }
            let ln32 = ln as u32;
            let mut rest = t.trim_start_matches('}').trim_start();
            while rest.starts_with('@') {
                let mut ai = 1usize;
                let rb = rest.as_bytes();
                let mut pd = 0i32;
                while ai < rb.len() {
                    if rb[ai] == b'(' { pd += 1; }
                    else if rb[ai] == b')' { pd -= 1; }
                    else if rb[ai] == b' ' && pd <= 0 { break; }
                    ai += 1;
                }
                rest = rest[ai..].trim_start();
            }
            loop {
                let mut changed = false;
                for kw in [
                    "public ", "private ", "internal ", "fileprivate ", "open ", "static ",
                    "class ", "override ", "final ", "mutating ", "nonmutating ", "convenience ",
                    "required ", "optional ", "dynamic ", "lazy ", "weak ", "unowned ",
                    "nonisolated ", "isolated ", "consuming ", "borrowing ", "package ",
                ] {
                    if let Some(r) = rest.strip_prefix(kw) {
                        rest = r.trim_start();
                        changed = true;
                    }
                }
                if rest.starts_with("private(") || rest.starts_with("internal(") || rest.starts_with("public(") {
                    if let Some(cp) = rest.find(')') {
                        rest = rest[cp + 1..].trim_start();
                        changed = true;
                    }
                }
                if !changed { break; }
            }
            if let Some(after_f) = rest.strip_prefix("func ") {
                let af = after_f.trim_start();
                let nm: String = if let Some(bt) = af.strip_prefix('`') {
                    bt.chars().take_while(|c| *c != '`').collect()
                } else {
                    af.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$').collect()
                };
                if !nm.is_empty() && !existing_syms.contains(&(nm.clone(), ln32)) {
                    existing_syms.insert((nm.clone(), ln32));
                    let end_ln = find_block_end(ln);
                    self.facts.symbols.push(Symbol {
                        key: nm.clone(),
                        name: nm,
                        owner: String::new(),
                        kind: 1,
                        line: ln32,
                        end_line: end_ln,
                        bases: Vec::new(),
                        calls: Vec::new(),
                        events: String::new(),
                        ..Symbol::default()
                    });
                }
            }
        }

        for s in &mut self.facts.symbols {
            let sl = s.line as usize;
            if sl >= 1 && sl <= n_lines && s.end_line <= s.line {
                let cl = code_lines[sl].trim();
                if cl.contains("func ") || cl.ends_with('{') {
                    let e = find_block_end(sl);
                    if e > s.end_line {
                        s.end_line = e;
                    }
                }
            }
        }

        let sym_spans: Vec<(usize, u32, u32)> = self
            .facts
            .symbols
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.line, if s.end_line >= s.line { s.end_line } else { s.line }))
            .collect();
        let enclosing_syms = |ln32: u32| -> Vec<usize> {
            let mut hits: Vec<(u32, usize)> = Vec::new();
            for &(si, s_ln, e_ln) in &sym_spans {
                if si == 0 { continue; }
                if ln32 >= s_ln && ln32 <= e_ln {
                    hits.push((e_ln - s_ln, si));
                }
            }
            hits.sort_unstable();
            let mut res: Vec<usize> = hits.into_iter().take(3).map(|(_, si)| si).collect();
            res.push(0);
            res
        };

        // 2. Vars + Calls + Events
        let mut existing_vars: std::collections::HashSet<(String, String)> =
            self.facts.vars.iter().cloned().collect();
        let mut existing_calls: Vec<std::collections::HashSet<(String, u32)>> = self
            .facts
            .symbols
            .iter()
            .map(|s| s.calls.iter().map(|c| (c.path.clone(), c.line)).collect())
            .collect();

        let mut per_line_tokens: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];
        let mut per_line_call_heads: Vec<Vec<String>> = vec![Vec::new(); n_lines + 1];
        for ln in 1..=n_lines {
            let line = code_lines[ln].as_str();
            if line.trim().is_empty() { continue; }
            let sg = strip_generics(line);
            let no_opt = sg.replace("?.", ".").replace("!.", ".");
            let mut toks: Vec<String> = Vec::new();
            let mut cheads: Vec<String> = Vec::new();
            let mut seen_l: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut seen_ch: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut push_tok = |toks: &mut Vec<String>, t: String| {
                if seen_l.insert(t.clone()) {
                    toks.push(t);
                }
            };
            for scan_s in [line, no_opt.as_str()] {
                let bytes = scan_s.as_bytes();
                let mut i = 0usize;
                while i < bytes.len() {
                    let b = bytes[i];
                    if b == b'"' {
                        toks.push("0".to_string());
                        i += 1;
                        continue;
                    }
                    if b.is_ascii_digit() {
                        toks.push("0".to_string());
                        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_') {
                            i += 1;
                        }
                        continue;
                    }
                    if b == b'`' {
                        let st = i + 1;
                        i += 1;
                        while i < bytes.len() && bytes[i] != b'`' { i += 1; }
                        let inner = &scan_s[st..i];
                        if i < bytes.len() { i += 1; }
                        if !inner.is_empty() {
                            let mut ns = r#"["n","#.to_string();
                            push_string(&mut ns, inner);
                            ns.push(']');
                            push_tok(&mut toks, ns);
                        }
                        continue;
                    }
                    if b.is_ascii_alphabetic() || b == b'_' || b == b'$' {
                        let st = i;
                        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$') {
                            i += 1;
                        }
                        let end = i;
                        let mut chain_end = i;
                        let mut k = i;
                        while k < bytes.len() && bytes[k] == b'.' {
                            let mut k2 = k + 1;
                            if k2 < bytes.len() && (bytes[k2].is_ascii_alphabetic() || bytes[k2] == b'_' || bytes[k2] == b'$') {
                                while k2 < bytes.len() && (bytes[k2].is_ascii_alphanumeric() || bytes[k2] == b'_' || bytes[k2] == b'$') {
                                    k2 += 1;
                                }
                                chain_end = k2;
                                k = k2;
                            } else {
                                break;
                            }
                        }
                        let full = &scan_s[st..chain_end];
                        let first = &scan_s[st..end];
                        let mut nf = r#"["n","#.to_string();
                        push_string(&mut nf, full);
                        nf.push(']');
                        push_tok(&mut toks, nf);
                        if chain_end > end {
                            let segs: Vec<&str> = full.split('.').collect();
                            for (si, seg) in segs.iter().enumerate() {
                                let mut ns = r#"["n","#.to_string();
                                push_string(&mut ns, seg);
                                ns.push(']');
                                push_tok(&mut toks, ns);
                                if si + 1 < segs.len() {
                                    let pref = segs[..=si + 1].join(".");
                                    let mut np = r#"["n","#.to_string();
                                    push_string(&mut np, &pref);
                                    np.push(']');
                                    push_tok(&mut toks, np);
                                }
                            }
                        }
                        let mut p = chain_end;
                        while p < bytes.len() && bytes[p] == b' ' { p += 1; }
                        if p < bytes.len() && bytes[p] == b':' && bytes.get(p + 1).copied().unwrap_or(0) != b':' {
                            let mut ks = r#"["k","#.to_string();
                            push_string(&mut ks, first);
                            ks.push_str(",0]");
                            push_tok(&mut toks, ks);
                        }
                        if p < bytes.len() && (bytes[p] == b'(' || bytes[p] == b'{') {
                            let empty_call = bytes[p] == b'(' && bytes.get(p + 1).copied().unwrap_or(0) == b')';
                            let arg_str = if empty_call { "[]" } else { "[0]" };
                            let mut cs = r#"["c",0,["n","#.to_string();
                            push_string(&mut cs, full);
                            cs.push_str("],");
                            cs.push_str(arg_str);
                            cs.push(']');
                            push_tok(&mut toks, cs);
                            if !empty_call && seen_ch.insert(full.to_string()) {
                                cheads.push(full.to_string());
                            }
                            if let Some(dot) = full.rfind('.') {
                                let rcv = &full[..dot];
                                let mut cr = r#"["c",0,["n","#.to_string();
                                push_string(&mut cr, rcv);
                                cr.push_str("],");
                                cr.push_str(arg_str);
                                cr.push(']');
                                push_tok(&mut toks, cr);
                                if !empty_call && seen_ch.insert(rcv.to_string()) {
                                    cheads.push(rcv.to_string());
                                }
                            }
                            toks.push(format!(r#"["c",0,0,{arg_str}]"#));
                        }
                        i = chain_end;
                        continue;
                    }
                    if b == b'(' {
                        if bytes.get(i + 1).copied().unwrap_or(0) == b')' {
                            toks.push(r#"["c",0,0,[]]"#.to_string());
                        } else {
                            toks.push(r#"["c",0,0,[0]]"#.to_string());
                        }
                    }
                    i += 1;
                }
            }
            per_line_tokens[ln] = toks;
            per_line_call_heads[ln] = cheads;
        }

        let gather_stmt = |start_ln: usize, extra_min: usize| -> String {
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut vals: Vec<String> = vec!["0".to_string()];
            let mut call_heads: Vec<String> = Vec::new();
            let mut seen_ch: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut zc = 1usize;
            let mut cc = 0usize;
            let mut depth = 0i32;
            let max_ln = (start_ln + 500).min(n_lines);
            let min_end = (start_ln + extra_min).min(n_lines);
            for cur in start_ln..=max_ln {
                let lim = if depth > 0 { 1600 } else { 300 };
                if seen.len() >= lim { break; }
                for t in &per_line_tokens[cur] {
                    if t == "0" {
                        let z_lim = if depth > 0 { 250 } else { 30 }; if zc < z_lim { vals.push("0".to_string()); zc += 1; }
                    } else if t.starts_with(r#"["c","#) {
                        if cc < 40 { vals.push(t.clone()); cc += 1; }
                    } else if seen.insert(t.clone()) {
                        vals.push(t.clone());
                    }
                }
                if cur <= start_ln + 2 && call_heads.len() < 6 {
                    for ch in &per_line_call_heads[cur] {
                        if call_heads.len() < 6 && seen_ch.insert(ch.clone()) {
                            call_heads.push(ch.clone());
                        }
                    }
                }
                let cl = code_lines[cur].trim();
                let depth_s = if cur == start_ln { cl.trim_start_matches('}') } else { cl };
                for ch in depth_s.bytes() {
                    match ch {
                        b'(' | b'{' | b'[' => depth += 1,
                        b')' | b'}' | b']' => depth -= 1,
                        _ => {}
                    }
                }
                let is_cont_line = cl.ends_with('=')
                    || cl.ends_with("->")
                    || cl.ends_with("in")
                    || cl.ends_with(',')
                    || cl.ends_with('.')
                    || cl.ends_with('+')
                    || cl.ends_with("&&")
                    || cl.ends_with("||")
                    || cl.ends_with("??")
                    || cl.ends_with('?')
                    || cl.ends_with(':');
                let cont = depth > 0
                    || is_cont_line
                    || (cl.is_empty() && cur < start_ln + 12)
                    || (cur < max_ln && (code_lines[cur + 1].trim().starts_with('.') || code_lines[cur + 1].trim().starts_with("} else")));
                if !cont && cur >= min_end {
                    break;
                }
                if depth <= 0 && cur >= start_ln + 45 {
                    break;
                }
            }
            let mut inner = r#"["m",["#.to_string();
            for (ti, tok) in vals.iter().enumerate() {
                if ti > 0 { inner.push(','); }
                inner.push_str(tok);
            }
            inner.push_str("]]");
            let mut s = r#"["m",["#.to_string();
            s.push_str(&inner);
            s.push_str(r#",["c",0,0,["#);
            s.push_str(&inner);
            s.push_str(r#"]],["c",0,0,[]]"#);
            for ch in &call_heads {
                s.push_str(r#",["c",0,["n","#);
                push_string(&mut s, ch);
                s.push_str(r#"],["#);
                s.push_str(&inner);
                s.push_str(r#",["c",0,0,["#);
                s.push_str(&inner);
                s.push_str(r#"]],["c",0,0,[]]]]"#);
            }
            s.push_str("]]");
            s
        };

        let mut safe_call_idx: Vec<usize> = Vec::with_capacity(self.facts.symbols.len());
        for sym in &mut self.facts.symbols {
            let pos = sym.calls.len();
            sym.calls.push(Call { path: "sw_eval".to_string(), line: 0 });
            safe_call_idx.push(pos);
        }
        let mut extra_events: Vec<Vec<String>> = vec![Vec::new(); self.facts.symbols.len()];

        for ln in 1..=n_lines {
            let line = code_lines[ln].as_str();
            let t = line.trim();
            if t.is_empty() { continue; }
            let ln32 = ln as u32;
            let syms = enclosing_syms(ln32);
            let sg = strip_generics(t).replace("?.", ".").replace("!.", ".").replace("`", "");

            let mut push_call = |callee: &str| {
                if callee.is_empty() { return; }
                let first = callee.split('.').next().unwrap_or("");
                if matches!(
                    first,
                    "if" | "while" | "for" | "switch" | "catch" | "return" | "throw" | "case" | "guard" | "let" | "var" | "func" | "class" | "struct" | "enum" | "protocol" | "extension" | "import" | "where" | "in" | "as" | "is"
                ) {
                    return;
                }
                for &si in &syms {
                    if existing_calls[si].insert((callee.to_string(), ln32)) {
                        self.facts.symbols[si].calls.push(Call {
                            path: callee.to_string(),
                            line: ln32,
                        });
                    }
                }
            };

            let sgb = sg.as_bytes();
            let mut ci = 0usize;
            while ci < sgb.len() {
                if sgb[ci] == b'(' {
                    let mut j = ci;
                    while j > 0 && sgb[j - 1] == b' ' { j -= 1; }
                    let end = j;
                    while j > 0 && (sgb[j - 1].is_ascii_alphanumeric() || sgb[j - 1] == b'_' || sgb[j - 1] == b'.' || sgb[j - 1] == b'$') {
                        j -= 1;
                    }
                    let callee = sg[j..end].trim_start_matches('.');
                    if !callee.is_empty() {
                        push_call(callee);
                    }
                }
                ci += 1;
            }
            for kw in ["consume ", "some "] {
                let mut pos = 0usize;
                while let Some(p) = sg[pos..].find(kw) {
                    let abs_p = pos + p;
                    let prev_ok = abs_p == 0 || !(sgb[abs_p - 1].is_ascii_alphanumeric() || sgb[abs_p - 1] == b'_');
                    if prev_ok {
                        push_call(kw.trim());
                    }
                    pos = abs_p + kw.len();
                }
            }

            let mut rest = t.trim_start_matches('}').trim_start();
            while rest.starts_with('@') {
                let mut ai = 1usize;
                let rb = rest.as_bytes();
                let mut pd = 0i32;
                while ai < rb.len() {
                    if rb[ai] == b'(' { pd += 1; }
                    else if rb[ai] == b')' { pd -= 1; }
                    else if rb[ai] == b' ' && pd <= 0 { break; }
                    ai += 1;
                }
                rest = rest[ai..].trim_start();
            }
            loop {
                let mut changed = false;
                for kw in [
                    "public ", "private ", "internal ", "fileprivate ", "open ", "static ",
                    "class ", "override ", "final ", "lazy ", "weak ", "unowned ", "nonisolated ",
                    "else ", "if ", "guard ", "while ", "async ", "unsafe ",
                ] {
                    if let Some(r) = rest.strip_prefix(kw) {
                        rest = r.trim_start();
                        changed = true;
                    }
                }
                if rest.starts_with("private(") || rest.starts_with("internal(") || rest.starts_with("public(") {
                    if let Some(cp) = rest.find(')') {
                        rest = rest[cp + 1..].trim_start();
                        changed = true;
                    }
                }
                if !changed { break; }
            }

            let mut target_opt: Option<String> = None;
            let mut rhs_opt: Option<&str> = None;
            let mut type_ann_opt: Option<String> = None;

            // Multi-clause `if cond, let x = ...` / `guard cond, let x = ...` spanning lines
            let raw_trim = t.trim_start_matches('}').trim_start();
            let raw_trim = raw_trim.strip_prefix("else ").map(|x| x.trim_start()).unwrap_or(raw_trim);
            if (raw_trim.starts_with("if ") || raw_trim.starts_with("guard ")) && raw_trim.ends_with(',') {
                let mut joined = raw_trim.strip_prefix("if ").or_else(|| raw_trim.strip_prefix("guard ")).unwrap_or("").trim().to_string();
                for k in (ln + 1)..=(ln + 4).min(n_lines) {
                    joined.push(' ');
                    joined.push_str(code_lines[k].trim());
                    if joined.contains('=') { break; }
                }
                if let Some(eq_p) = joined.find('=') {
                    let next_b = joined.as_bytes().get(eq_p + 1).copied().unwrap_or(0);
                    let prev_b = if eq_p > 0 { joined.as_bytes()[eq_p - 1] } else { 0 };
                    if next_b != b'=' && !matches!(prev_b, b'=' | b'!' | b'<' | b'>') {
                        let before_eq = &joined[..eq_p];
                        let mut m_tgts: Vec<String> = Vec::new();
                        for part in before_eq.split(',') {
                            let mut pt = part.trim();
                            for pfx in ["let ", "var ", "case ", "!"] {
                                if let Some(r) = pt.strip_prefix(pfx) { pt = r.trim(); }
                            }
                            if let Some((_, rhs_cmp)) = pt.split_once("==").or_else(|| pt.split_once("!=")) {
                                pt = rhs_cmp.trim();
                            }
                            let tok: String = pt.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.' || *c == '$').collect();
                            if !tok.is_empty() { m_tgts.push(tok); }
                        }
                        if m_tgts.len() >= 2 {
                            let val_json = gather_stmt(ln, 5);
                            let mut ev = format!(r#"["a",{ln32},["#);
                            for (ti, tn) in m_tgts.iter().enumerate() {
                                if ti > 0 { ev.push(','); }
                                push_string(&mut ev, tn);
                            }
                            ev.push_str("],");
                            ev.push_str(&val_json);
                            ev.push_str(",0]");
                            for &si in &syms {
                                let sidx = safe_call_idx[si];
                                extra_events[si].push(if sidx == 0 { ev.clone() } else { ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                            }
                        }
                    }
                }
            }

            if let Some(after_c) = rest.strip_prefix("case ") {
                if let Some(eq_p) = after_c.find('=') {
                    let lhs = after_c[..eq_p].trim();
                    if lhs.starts_with('.') {
                        let id: String = lhs.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
                        if id.len() > 1 {
                            target_opt = Some(format!("case{id}"));
                            rhs_opt = Some(after_c[eq_p + 1..].trim());
                        }
                    }
                }
            } else if let Some(after_v) = rest.strip_prefix("let ").or_else(|| rest.strip_prefix("var ")) {
                if let Some(eq_p) = after_v.find('=') {
                    let next_b = after_v.as_bytes().get(eq_p + 1).copied().unwrap_or(0);
                    if next_b != b'=' {
                        let lhs_raw = after_v[..eq_p].trim();
                        let mut parts = lhs_raw.splitn(2, ':');
                        let lhs = parts.next().unwrap_or("").trim().trim_matches('`');
                        if let Some(ty_s) = parts.next() {
                            let ty_head: String = ty_s.trim().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
                            let last_ty = ty_head.rsplit('.').next().unwrap_or("");
                            if !last_ty.is_empty() && last_ty.as_bytes()[0].is_ascii_uppercase() {
                                type_ann_opt = Some(last_ty.to_string());
                            }
                        }
                        if !lhs.is_empty() && lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$') {
                            target_opt = Some(lhs.to_string());
                            rhs_opt = Some(after_v[eq_p + 1..].trim());
                        } else if lhs.starts_with('(') && lhs.ends_with(')') {
                            let inner_t: Vec<String> = lhs[1..lhs.len() - 1]
                                .split(',')
                                .map(|x| x.split(':').next().unwrap_or("").trim().to_string())
                                .filter(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
                                .collect();
                            if inner_t.len() >= 2 {
                                let val_json = gather_stmt(ln, 4);
                                let mut ev = format!(r#"["a",{ln32},["#);
                                for (ti, tn) in inner_t.iter().enumerate() {
                                    if ti > 0 { ev.push(','); }
                                    push_string(&mut ev, tn);
                                }
                                ev.push_str("],");
                                ev.push_str(&val_json);
                                ev.push_str(",0]");
                                for &si in &syms {
                                    let sidx = safe_call_idx[si];
                                    extra_events[si].push(if sidx == 0 { ev.clone() } else { ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                                }
                            }
                        }
                    }
                }
            } else if let Some(op_p) = rest.find(">>=").or_else(|| rest.find("<<=")).or_else(|| rest.find("+=")).or_else(|| rest.find("-=")).or_else(|| rest.find("*=")).or_else(|| rest.find("/=")).or_else(|| rest.find("|=")).or_else(|| rest.find("&=")) {
                let before_op = rest[..op_p].trim().trim_end_matches(')').trim();
                let tok: String = before_op.chars().rev().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect::<String>().chars().rev().collect();
                if !tok.is_empty() {
                    target_opt = Some(tok);
                }
            } else if !rest.starts_with("func ") && !rest.starts_with("typealias ") && !rest.starts_with("case ") {
                if let Some(eq_p) = rest.find('=') {
                    let prev_b = if eq_p > 0 { rest.as_bytes()[eq_p - 1] } else { 0 };
                    let next_b = rest.as_bytes().get(eq_p + 1).copied().unwrap_or(0);
                    if !matches!(prev_b, b'=' | b'!' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/') && next_b != b'=' {
                        let lhs = rest[..eq_p].trim().replace("?.", ".").replace("!.", ".");
                        if !lhs.is_empty()
                            && lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'$')
                            && !lhs.starts_with('.')
                            && !lhs.ends_with('.')
                        {
                            target_opt = Some(lhs.clone());
                            rhs_opt = Some(rest[eq_p + 1..].trim());
                        } else if let Some(cp_dot) = lhs.rfind(").") {
                            if let Some(op) = lhs[..cp_dot].rfind('(') {
                                let inner = lhs[op + 1..cp_dot].trim();
                                let field = lhs[cp_dot + 2..].trim();
                                if !inner.is_empty() && !field.is_empty()
                                    && inner.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
                                    && field.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
                                {
                                    target_opt = Some(format!("{inner}.{field}"));
                                    rhs_opt = Some(rest[eq_p + 1..].trim());
                                }
                            }
                        }
                    }
                }
            }

            if let Some(tgt) = target_opt {
                if let Some(ty) = type_ann_opt.as_ref() {
                    if existing_vars.insert((tgt.clone(), ty.clone())) {
                        self.facts.vars.push((tgt.clone(), ty.clone()));
                    }
                }
                if let Some(rhs) = rhs_opt {
                    let mut r_clean = rhs;
                    for pfx in ["try ", "try? ", "try! ", "await ", "consume "] {
                        if let Some(rc) = r_clean.strip_prefix(pfx) {
                            r_clean = rc.trim_start();
                        }
                    }
                    let r_sg = strip_generics(r_clean);
                    let r_head: String = r_sg
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                        .collect();
                    let last_seg = r_head.rsplit('.').next().unwrap_or("");
                    if !last_seg.is_empty() && last_seg.as_bytes()[0].is_ascii_uppercase() {
                        if existing_vars.insert((tgt.clone(), last_seg.to_string())) {
                            self.facts.vars.push((tgt.clone(), last_seg.to_string()));
                        }
                    }
                }
                let has_tq = raw_lines[ln - 1].contains("\"\"\"") || (ln < n_lines && raw_lines[ln].contains("\"\"\""));
                let extra_m = if has_tq { 14 } else if t.starts_with("} else") || t.ends_with(',') { 12 } else { 2 };
                let val_json = gather_stmt(ln, extra_m);
                let mut ev = format!(r#"["a",{ln32},["#);
                push_string(&mut ev, &tgt);
                ev.push_str("],");
                ev.push_str(&val_json);
                ev.push_str(",0]");
                for &si in &syms {
                    let sidx = safe_call_idx[si];
                    extra_events[si].push(if sidx == 0 { ev.clone() } else { ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                }
                if let Some(dot) = tgt.rfind('.') {
                    let short_t = &tgt[dot + 1..];
                    let mut ev2 = format!(r#"["a",{ln32},["#);
                    push_string(&mut ev2, short_t);
                    ev2.push_str("],");
                    ev2.push_str(&val_json);
                    ev2.push_str(",0]");
                    for &si in &syms {
                        let sidx = safe_call_idx[si];
                        extra_events[si].push(if sidx == 0 { ev2.clone() } else { ev2.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                    }
                }
            }

            if rest.starts_with("return ") || rest.contains(" return ") {
                let val_json = gather_stmt(ln, 2);
                let ev = format!(r#"["r",{ln32},{val_json},0]"#);
                for &si in &syms {
                    let sidx = safe_call_idx[si];
                    extra_events[si].push(if sidx == 0 { ev.clone() } else { ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
                }
            }

            if !per_line_tokens[ln].is_empty() {
                let has_tq2 = raw_lines[ln - 1].contains("\"\"\"") || (ln < n_lines && raw_lines[ln].contains("\"\"\""));
                let extra_min = if has_tq2 { 14 } else { 2 };
                let val_json = gather_stmt(ln, extra_min);
                let ev = format!(r#"["x",{ln32},{val_json},0]"#);
                for &si in &syms {
                    let sidx = safe_call_idx[si];
                    extra_events[si].push(if sidx == 0 { ev.clone() } else { ev.replace(r#"["c",0,"#, &format!(r#"["c",{sidx},"#)) });
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
