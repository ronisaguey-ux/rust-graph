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
        let mut p_depth = 0i32;
        let mut b_depth = 0i32;
        let mut eq_at = None;
        for (i, &n) in stmt.iter().enumerate() {
            match self.kind(n) {
                "(" | "[" => p_depth += 1,
                ")" | "]" => p_depth = (p_depth - 1).max(0),
                "{" => b_depth += 1,
                "}" => b_depth = (b_depth - 1).max(0),
                "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | "??="
                    if p_depth == 0 && b_depth == 0 =>
                {
                    eq_at = Some(i);
                    break;
                }
                _ => {}
            }
        }
        if let Some(at) = eq_at {
            if at > 0 {
                let targets = self.cs_recovered_targets(&stmt[..at]);
                if !targets.is_empty() {
                    let value = self.cs_recovered_form(&stmt[at + 1..]);
                    let weak = self.kind(stmt[at]) != "="
                        || stmt[..at].last().is_some_and(|&n| {
                            matches!(self.kind(n), "]" | "bracketed_argument_list" | "element_access")
                        });
                    self.lexer_event(b'a', line, &targets, &value, weak);
                    return;
                }
            }
        }
        // A class/struct/interface/enum/record header stranded in an ERROR node
        // still declares a type symbol for the one-pass reader.
        if let Some(at) = stmt
            .iter()
            .position(|&n| matches!(self.kind(n), "class" | "struct" | "interface" | "enum" | "record"))
        {
            if stmt[..at].iter().all(|&n| {
                matches!(
                    self.kind(n),
                    "public" | "private" | "protected" | "internal" | "static" | "sealed"
                        | "abstract" | "partial" | "new" | "readonly" | "ref" | "file"
                )
            }) {
                if let Some(&name_node) = stmt.get(at + 1) {
                    if let Some(name) = self.cs_leaf_ident(name_node) {
                        if !self
                            .facts
                            .symbols
                            .iter()
                            .any(|s| s.name == name && s.line == line && s.kind == KIND_CLASS)
                        {
                            let owner = self.frame().class.clone();
                            self.add_symbol(name, owner, KIND_CLASS, line, Vec::new());
                        }
                    }
                }
            }
        }
        let value = self.cs_recovered_form(stmt);
        if value != "0" && !value.is_empty() {
            self.lexer_event(b'x', line, &[], &value, false);
        }
    }

    fn cs_recovered_targets(&self, mut lhs: &[u32]) -> Vec<String> {
        // `Inner[key] = value` / `this[index] = value`: strip trailing `[...]`
        // subscripts so the target is the receiver, not the index expression.
        while let Some(&last) = lhs.last() {
            if self.kind(last) == "bracketed_argument_list" {
                lhs = &lhs[..lhs.len() - 1];
                continue;
            }
            if self.kind(last) != "]" {
                break;
            }
            let mut depth = 0i32;
            let mut open = None;
            for (i, &n) in lhs.iter().enumerate().rev() {
                match self.kind(n) {
                    "]" => depth += 1,
                    "[" => {
                        depth -= 1;
                        if depth == 0 {
                            open = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(i) = open else { break };
            lhs = &lhs[..i];
        }
        if lhs.len() == 1 && self.kind(lhs[0]) == "element_access" {
            return self.assignment_targets(lhs[0]);
        }
        // `var (a, b) = ...` / `(a, b) = ...`: extract every binder in the tuple.
        if lhs.last().is_some_and(|&n| self.kind(n) == ")") {
            let mut depth = 0i32;
            let mut open = None;
            for (i, &n) in lhs.iter().enumerate().rev() {
                match self.kind(n) {
                    ")" => depth += 1,
                    "(" => {
                        depth -= 1;
                        if depth == 0 {
                            open = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(i) = open {
                if i == 0 || (i == 1 && matches!(self.kind(lhs[0]), "var" | "ref")) {
                    let inner = &lhs[i + 1..lhs.len() - 1];
                    let mut out = Vec::new();
                    let mut start = 0usize;
                    let mut p = 0i32;
                    for (j, &n) in inner.iter().enumerate() {
                        match self.kind(n) {
                            "(" | "[" | "<" => p += 1,
                            ")" | "]" | ">" => p = (p - 1).max(0),
                            "," if p == 0 => {
                                out.extend(self.cs_recovered_targets(&inner[start..j]));
                                start = j + 1;
                            }
                            _ => {}
                        }
                    }
                    if start < inner.len() {
                        out.extend(self.cs_recovered_targets(&inner[start..]));
                    }
                    if !out.is_empty() {
                        return out;
                    }
                }
            }
        }
        let mut is_dotted = !lhs.is_empty();
        let mut segs: Vec<String> = Vec::new();
        for (i, &n) in lhs.iter().enumerate() {
            if i % 2 == 1 {
                if !matches!(self.kind(n), "." | "?.") {
                    is_dotted = false;
                    break;
                }
            } else if let Some(name) = self.cs_leaf_ident(n) {
                segs.push(if name == "this" { "self".to_string() } else { name });
            } else {
                is_dotted = false;
                break;
            }
        }
        if is_dotted && !segs.is_empty() && lhs.len() % 2 == 1 {
            return vec![segs.join(".")];
        }
        lhs.iter()
            .rev()
            .find_map(|&n| self.cs_leaf_ident(n))
            .map(|name| if name == "this" { "self".to_string() } else { name })
            .into_iter()
            .collect()
    }

    fn cs_leaf_ident(&self, id: u32) -> Option<String> {
        let kind = self.kind(id);
        if kind == "identifier"
            || self.kinds.soft.contains(&kind)
            || matches!(kind, "file" | "set" | "remove" | "get" | "init" | "add" | "value" | "var" | "this")
        {
            Some(self.text_string(id))
        } else {
            self.leaf_name(id)
        }
    }

    fn cs_recovered_form(&mut self, nodes: &[u32]) -> String {
        let symbol = self.frame().symbol;
        let calls_len = self.facts.symbols[symbol].calls.len();
        let mut parts: Vec<String> = Vec::new();
        let mut i = 0;
        while i < nodes.len() {
            let n = nodes[i];
            if self.node(n).first_child != NONE {
                let f = self.form(n);
                if !f.is_empty() && f != "0" {
                    parts.push(f);
                }
                i += 1;
                continue;
            }
            let kind = self.kind(n);
            if matches!(kind, "true" | "false" | "null") {
                parts.push(format!("[\"n\",\"{kind}\"]"));
                i += 1;
                continue;
            }
            if matches!(
                kind,
                "integer_literal" | "real_literal" | "string_literal" | "character_literal"
                    | "verbatim_string_literal" | "raw_string_literal"
            ) {
                parts.push("0".to_string());
                i += 1;
                continue;
            }
            if let Some(first) = self.cs_leaf_ident(n) {
                if matches!(
                    first.as_str(),
                    "var" | "await" | "out" | "ref" | "in" | "is" | "as" | "new" | "throw" | "yield"
                ) {
                    i += 1;
                    continue;
                }
                let is_new = i > 0 && self.kind(nodes[i - 1]) == "new";
                let mut segs = vec![if first == "this" { "self".to_string() } else { first }];
                let mut end = i;
                while end + 2 < nodes.len() && matches!(self.kind(nodes[end + 1]), "." | "?." | "::") {
                    if let Some(seg) = self.cs_leaf_ident(nodes[end + 2]) {
                        segs.push(seg);
                        end += 2;
                    } else {
                        break;
                    }
                }
                let mut after = end + 1;
                if after < nodes.len() && self.kind(nodes[after]) == "<" {
                    let mut depth = 0i32;
                    let mut j = after;
                    while j < nodes.len() {
                        match self.kind(nodes[j]) {
                            "<" => depth += 1,
                            ">" | ">joined" => {
                                depth -= 1;
                                if depth <= 0 {
                                    after = j + 1;
                                    break;
                                }
                            }
                            ";" | "{" | "}" | "(" => break,
                            _ => {}
                        }
                        j += 1;
                    }
                }
                if after < nodes.len() && self.kind(nodes[after]) == "(" {
                    let mut depth = 0i32;
                    let mut close = None;
                    for (j, &tok) in nodes.iter().enumerate().skip(after) {
                        match self.kind(tok) {
                            "(" => depth += 1,
                            ")" => {
                                depth -= 1;
                                if depth == 0 {
                                    close = Some(j);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    if let Some(close_idx) = close {
                        let mut args: Vec<String> = Vec::new();
                        let mut arg_start = after + 1;
                        let mut p = 0i32;
                        let mut b = 0i32;
                        for j in (after + 1)..close_idx {
                            match self.kind(nodes[j]) {
                                "(" | "[" => p += 1,
                                ")" | "]" => p = (p - 1).max(0),
                                "{" => b += 1,
                                "}" => b = (b - 1).max(0),
                                "," if p == 0 && b == 0 => {
                                    args.push(self.cs_recovered_form(&nodes[arg_start..j]));
                                    arg_start = j + 1;
                                }
                                _ => {}
                            }
                        }
                        if arg_start < close_idx {
                            args.push(self.cs_recovered_form(&nodes[arg_start..close_idx]));
                        }
                        let recv = if !is_new && segs.len() > 1 {
                            let recv_name = segs[..segs.len() - 1].join(".");
                            let mut r = String::from("[\"n\",");
                            push_string(&mut r, &recv_name);
                            r.push(']');
                            r
                        } else {
                            "0".to_string()
                        };
                        let call_name = segs.join(".");
                        let call_line = self.line(nodes[end]);
                        let call_idx = self.facts.symbols[symbol]
                            .calls
                            .iter()
                            .position(|c| c.line == call_line && c.path == call_name)
                            .unwrap_or(0);
                        parts.push(format!("[\"c\",{call_idx},{recv},[{}]]", args.join(",")));
                        i = close_idx + 1;
                        continue;
                    }
                }
                let name = segs.join(".");
                let mut f = String::from("[\"n\",");
                push_string(&mut f, &name);
                f.push(']');
                parts.push(f);
                i = end + 1;
                continue;
            }
            i += 1;
        }
        self.facts.symbols[symbol].calls.truncate(calls_len);
        // Drop bare `0` literals when combined with names/calls in an expression,
        // matching how `form` omits constants inside merges, while keeping `0`
        // when it is the sole value (e.g. a call argument or `return 0;`).
        if parts.len() > 1 && parts.iter().any(|p| p != "0") {
            parts.retain(|p| p != "0");
        }
        match parts.len() {
            0 => "0".to_string(),
            1 => parts.pop().unwrap(),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        }
    }

    pub(crate) fn push_call(&mut self, path: String, line: u32) -> usize {
        let symbol = self.frame().symbol;
        let calls = &mut self.facts.symbols[symbol].calls;
        calls.push(Call { path, line });
        calls.len() - 1
    }

    /// `new T { Name = value, ... }`: the one-pass reader writes one `a` event for
    /// the initialiser, on its first member's line, targeting that member. The
    /// other members are folded into its value as names, so a later member is
    /// not written on a line of its own. The CST read the initialiser for calls
    /// alone and dropped that line, which left every `new T { ... }` (Avalonia's
    /// `FilePickerFileTypes`, 20+ rows in one corpus) without the event.
    pub(crate) fn cs_initializer_first_member(&mut self, id: u32) {
        let children = self.children(id);
        let mut members: Vec<(u32, u32)> = Vec::new();
        let mut at = 0;
        while at < children.len() {
            if self.kind(children[at]) == "identifier" {
                if let Some(&value) = children[at + 1..].iter().find(|&&child| self.kind(child) != "=") {
                    members.push((children[at], value));
                    at = children.iter().position(|&child| child == value).unwrap_or(at) + 1;
                    continue;
                }
            }
            at += 1;
        }
        let Some(&(name_node, value_node)) = members.first() else {
            return;
        };
        let name = self.text_string(name_node);
        let line = self.line(name_node);
        let value = if members.len() == 1 {
            self.cs_member_value(value_node)
        } else {
            // The later members, as the lexer folds them: a number is only its
            // member's name, anything else is the name merged with its value, so
            // every name and keyword literal the lexer read is an atom here too.
            let mut parts = vec![self.cs_member_value(value_node)];
            for &(later_name, later_value) in &members[1..] {
                let later = self.text_string(later_name);
                if self.kind(later_value).ends_with("integer_literal") {
                    parts.push(format!("[\"n\",\"{later}\"]"));
                } else {
                    parts.push(format!("[\"m\",[[\"n\",\"{later}\"],{}]]", self.cs_member_value(later_value)));
                }
            }
            format!("[\"t\",[{}]]", parts.join(","))
        };
        self.lexer_event(b'a', line, &[name], &value, false);
        // The one-pass reader binds every member to the class it constructs, as
        // it does a declaration (`var Background = SolidColorBrush`), so each
        // member gets the same binding.
        for (name_node, value_node) in members {
            let name = self.text_string(name_node);
            self.cs_bind_constructed(name, value_node);
        }
    }

    /// The value of one initialiser member: a keyword literal is a name to the
    /// one-pass reader, as in a declaration's initialiser, and anything else is
    /// its own form (a constant when the form is empty). The calls the form
    /// reads are already facts of the object-initialiser walk, so they are
    /// taken back out of the call table: the form is only the value's shape.
    fn cs_member_value(&mut self, value_node: u32) -> String {
        match self.kind(value_node) {
            kind @ ("true" | "false" | "null") => format!("[\"n\",\"{kind}\"]"),
            _ => {
                let symbol = self.frame().symbol;
                let calls = self.facts.symbols[symbol].calls.len();
                let form = self.form(value_node);
                self.facts.symbols[symbol].calls.truncate(calls);
                if form.is_empty() { "0".to_string() } else { form }
            }
        }
    }

    // -- enter / leave ------------------------------------------------------


    fn tcl_is_ident(s: &str) -> bool {
        s.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b':' || b == b'.')
    }

    fn tcl_clean_name(s: &str) -> String {
        s.trim_start_matches("::").replace("::", ".")
    }

    fn tcl_var_name(s: &str) -> Option<String> {
        let rest = s.strip_prefix('$')?;
        let rest = if let Some(after) = rest.strip_prefix('{') {
            after.split('}').next().unwrap_or(after)
        } else {
            rest
        };
        let rest = rest.trim_start_matches("::");
        let base = rest.split('(').next().unwrap_or(rest);
        let name: String = base.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == ':').collect();
        let clean = Self::tcl_clean_name(&name);
        if !clean.is_empty() && clean.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
            Some(clean)
        } else {
            None
        }
    }

    fn tcl_tokenize_commands(text: &str, start_line: u32) -> Vec<(u32, Vec<(u32, String)>)> {
        let bytes = text.as_bytes();
        let mut cmds: Vec<(u32, Vec<(u32, String)>)> = Vec::new();
        let mut cur: Vec<(u32, String)> = Vec::new();
        let mut cmd_line = start_line;
        let mut line = start_line;
        let mut i = 0;
        let n = bytes.len();
        while i < n {
            let b = bytes[i];
            if b == b' ' || b == b'\t' || b == b'\r' {
                i += 1;
                continue;
            }
            if b == b'\\' && i + 1 < n && bytes[i + 1] == b'\n' {
                i += 2;
                line += 1;
                continue;
            }
            if b == b'\n' || b == b';' {
                if !cur.is_empty() {
                    cmds.push((cmd_line, std::mem::take(&mut cur)));
                }
                if b == b'\n' {
                    line += 1;
                }
                i += 1;
                continue;
            }
            if b == b'#' && cur.is_empty() {
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if cur.is_empty() {
                cmd_line = line;
            }
            let tok_line = line;
            let start = i;
            if b == b'{' {
                let mut depth = 0usize;
                while i < n {
                    if bytes[i] == b'\\' && i + 1 < n {
                        if bytes[i + 1] == b'\n' {
                            line += 1;
                        }
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'\n' {
                        line += 1;
                    } else if bytes[i] == b'{' {
                        depth += 1;
                    } else if bytes[i] == b'}' {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    i += 1;
                }
            } else if b == b'[' {
                let mut depth = 0usize;
                while i < n {
                    if bytes[i] == b'\\' && i + 1 < n {
                        if bytes[i + 1] == b'\n' {
                            line += 1;
                        }
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'\n' {
                        line += 1;
                    } else if bytes[i] == b'[' {
                        depth += 1;
                    } else if bytes[i] == b']' {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    i += 1;
                }
            } else if b == b'"' {
                i += 1;
                while i < n {
                    if bytes[i] == b'\\' && i + 1 < n {
                        if bytes[i + 1] == b'\n' {
                            line += 1;
                        }
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'\n' {
                        line += 1;
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            } else {
                while i < n && !matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b';' | b'[') {
                    if bytes[i] == b'\\' && i + 1 < n {
                        if bytes[i + 1] == b'\n' {
                            break;
                        }
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
            }
            if i > start {
                cur.push((tok_line, text[start..i].to_string()));
            } else {
                i += 1;
            }
        }
        if !cur.is_empty() {
            cmds.push((cmd_line, cur));
        }
        cmds
    }

    fn tcl_eval_words(&mut self, words: &[(u32, String)], cmd_line: u32, depth: usize) -> String {
        if words.is_empty() || depth > 24 {
            return String::new();
        }
        // Handle nested `proc name args body`
        if words[0].1 == "proc" && words.len() >= 2 {
            let args_braced = words.get(2).is_some_and(|w| w.1.starts_with('{'));
            let body_braced = words.len() >= 4 && words.last().is_some_and(|w| w.1.starts_with('{'));
            if !args_braced || !body_braced {
                self.push_call("proc".to_string(), cmd_line);
            }
            let raw_name = words[1].1.trim_matches(|c| c == '{' || c == '}');
            let sym_name = Self::tcl_clean_name(raw_name);
            if !sym_name.is_empty() {
                let sym = self.add_symbol(sym_name, String::new(), KIND_FUNCTION, cmd_line, Vec::new());
                self.stack.push(Frame {
                    node: 0,
                    symbol: sym,
                    class: String::new(),
                    receiver: String::new(),
                    container: false,
                    defines: Some(sym),
                });
                if let Some((bline, body)) = words.last() {
                    if words.len() >= 4 && body.starts_with('{') && body.ends_with('}') && body.len() >= 2 {
                        let inner = &body[1..body.len() - 1];
                        self.tcl_scan_script(inner, *bline, depth + 1);
                    }
                }
                self.stack.pop();
                return String::new();
            }
        }
        // Scan braced_word bodies recursively
        // Tuple: (wline, ident_opt, form_str, is_value_token)
        // Only plain words, quoted strings, and $variables are value tokens that trigger juxtaposed command_calls
        let mut word_forms: Vec<(u32, Option<String>, String, bool)> = Vec::new();
        for (wline, w) in words {
            if w.starts_with('{') && w.ends_with('}') && w.len() >= 2 {
                let inner = &w[1..w.len() - 1];
                self.tcl_scan_script(inner, *wline, depth + 1);
                let bform = if !inner.contains('\n') {
                    let bsegs: Vec<String> = inner
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != ':')
                        .map(|s| Self::tcl_clean_name(s))
                        .filter(|s| Self::tcl_is_ident(s))
                        .collect();
                    if bsegs.is_empty() {
                        "0".to_string()
                    } else if bsegs.len() == 1 {
                        let mut n = String::from("[\"n\",");
                        push_string(&mut n, &bsegs[0]);
                        n.push(']');
                        n
                    } else {
                        let parts: Vec<String> = bsegs
                            .iter()
                            .map(|s| {
                                let mut n = String::from("[\"n\",");
                                push_string(&mut n, s);
                                n.push(']');
                                n
                            })
                            .collect();
                        format!("[\"m\",[{}]]", parts.join(","))
                    }
                } else {
                    "0".to_string()
                };
                word_forms.push((*wline, None, bform, false));
            } else if w.starts_with('[') && w.ends_with(']') && w.len() >= 2 {
                let inner = &w[1..w.len() - 1];
                let sub_cmds = Self::tcl_tokenize_commands(inner, *wline);
                let mut last_form = "0".to_string();
                for (sline, swords) in sub_cmds {
                    let f = self.tcl_eval_words(&swords, sline, depth + 1);
                    if !f.is_empty() {
                        last_form = f;
                    }
                }
                word_forms.push((*wline, None, format!("[\"t\",[{last_form}]]"), false));
            } else if w.starts_with('"') && w.ends_with('"') && w.len() >= 2 {
                // Scan any [...] inside quoted string
                let inner = &w[1..w.len() - 1];
                let ib = inner.as_bytes();
                let mut j = 0;
                let mut cur_line = *wline;
                while j < ib.len() {
                    if ib[j] == b'\\' && j + 1 < ib.len() {
                        if ib[j + 1] == b'\n' {
                            cur_line += 1;
                        }
                        j += 2;
                        continue;
                    }
                    if ib[j] == b'\n' {
                        cur_line += 1;
                    } else if ib[j] == b'[' {
                        let bstart = j + 1;
                        let bline = cur_line;
                        let mut d = 1usize;
                        j += 1;
                        while j < ib.len() && d > 0 {
                            if ib[j] == b'\\' && j + 1 < ib.len() {
                                if ib[j + 1] == b'\n' {
                                    cur_line += 1;
                                }
                                j += 2;
                                continue;
                            }
                            if ib[j] == b'\n' {
                                cur_line += 1;
                            } else if ib[j] == b'[' {
                                d += 1;
                            } else if ib[j] == b']' {
                                d -= 1;
                                if d == 0 {
                                    let sub = &inner[bstart..j];
                                    for (sline, swords) in Self::tcl_tokenize_commands(sub, bline) {
                                        let _ = self.tcl_eval_words(&swords, sline, depth + 1);
                                    }
                                }
                            }
                            j += 1;
                        }
                        continue;
                    }
                    j += 1;
                }
                // Also extract $var interpolations inside quoted_word
                let mut qvars: Vec<String> = Vec::new();
                let mut qj = 0;
                while qj < ib.len() {
                    if ib[qj] == b'\\' && qj + 1 < ib.len() {
                        qj += 2;
                        continue;
                    }
                    if ib[qj] == b'$' {
                        let rest = &inner[qj..];
                        if let Some(vn) = Self::tcl_var_name(rest) {
                            let mut n = String::from("[\"n\",");
                            push_string(&mut n, &vn);
                            n.push(']');
                            qvars.push(n);
                        }
                    }
                    qj += 1;
                }
                let qform = if qvars.is_empty() {
                    "0".to_string()
                } else if qvars.len() == 1 {
                    qvars.remove(0)
                } else {
                    format!("[\"m\",[{}]]", qvars.join(","))
                };
                word_forms.push((*wline, None, qform, true));
            } else if let Some(vname) = Self::tcl_var_name(w) {
                if let Some((_, rest)) = w.split_once('(') {
                    let cid = self.push_call(vname.clone(), *wline);
                    let inner = rest.trim_end_matches(')');
                    let isegs: Vec<String> = inner
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != ':')
                        .map(|s| Self::tcl_clean_name(s))
                        .filter(|s| Self::tcl_is_ident(s))
                        .collect();
                    let arg_form = if isegs.is_empty() {
                        "0".to_string()
                    } else if isegs.len() == 1 {
                        let mut an = String::from("[\"n\",");
                        push_string(&mut an, &isegs[0]);
                        an.push(']');
                        an
                    } else {
                        let parts: Vec<String> = isegs
                            .iter()
                            .map(|s| {
                                let mut an = String::from("[\"n\",");
                                push_string(&mut an, s);
                                an.push(']');
                                an
                            })
                            .collect();
                        format!("[\"m\",[{}]]", parts.join(","))
                    };
                    let form = format!("[\"c\",{cid},0,[{arg_form}]]");
                    word_forms.push((*wline, Some(vname), form, true));
                } else {
                    let mut n = String::from("[\"n\",");
                    push_string(&mut n, &vname);
                    n.push(']');
                    word_forms.push((*wline, Some(vname), n, true));
                }
            } else if w.starts_with("::") {
                let clean = Self::tcl_clean_name(w);
                if Self::tcl_is_ident(&clean) {
                    // Join leading ::name to previous identifier with '.' as reference lexer does
                    if let Some(prev) = word_forms.last_mut() {
                        if let Some(ref mut prev_id) = prev.1 {
                            prev_id.push('.');
                            prev_id.push_str(&clean);
                            let mut n = String::from("[\"n\",");
                            push_string(&mut n, prev_id);
                            n.push(']');
                            prev.2 = n;
                            continue;
                        }
                    }
                    let mut n = String::from("[\"n\",");
                    push_string(&mut n, &clean);
                    n.push(']');
                    word_forms.push((*wline, Some(clean), n, true));
                } else {
                    word_forms.push((*wline, None, "0".to_string(), false));
                }
            } else {
                // Handle array element like env(FOO), RunTimes(command), hooks($curName)
                if let Some((base, rest)) = w.split_once('(') {
                    let clean = Self::tcl_clean_name(base);
                    if Self::tcl_is_ident(&clean) {
                        let cid = self.push_call(clean.clone(), *wline);
                        let inner = rest.trim_end_matches(')');
                        let arg_form = if let Some(vn) = Self::tcl_var_name(inner) {
                            let mut an = String::from("[\"n\",");
                            push_string(&mut an, &vn);
                            an.push(']');
                            an
                        } else {
                            let ic = Self::tcl_clean_name(inner);
                            if Self::tcl_is_ident(&ic) {
                                let mut an = String::from("[\"n\",");
                                push_string(&mut an, &ic);
                                an.push(']');
                                an
                            } else {
                                "0".to_string()
                            }
                        };
                        let form = format!("[\"c\",{cid},0,[{arg_form}]]");
                        word_forms.push((*wline, Some(clean), form, true));
                    } else {
                        word_forms.push((*wline, None, "0".to_string(), false));
                    }
                } else {
                    // Handle -flag, hyphenated words (amazon-s3), and slash/dot paths (tcl/httpd)
                    let is_flag = w.starts_with('-') && w.len() > 1 && w.as_bytes()[1].is_ascii_alphabetic();
                    let raw_tail = w.rsplit('-').next().unwrap_or(w);
                    let clean = Self::tcl_clean_name(raw_tail);
                    // Extract all identifier segments so hyphen/slash words never lose names
                    let segs: Vec<String> = w
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != ':')
                        .map(|s| Self::tcl_clean_name(s))
                        .filter(|s| Self::tcl_is_ident(s))
                        .collect();
                    let seg_form = if segs.is_empty() {
                        "0".to_string()
                    } else if segs.len() == 1 {
                        let mut n = String::from("[\"n\",");
                        push_string(&mut n, &segs[0]);
                        n.push(']');
                        n
                    } else {
                        let parts: Vec<String> = segs
                            .iter()
                            .map(|s| {
                                let mut n = String::from("[\"n\",");
                                push_string(&mut n, s);
                                n.push(']');
                                n
                            })
                            .collect();
                        format!("[\"m\",[{}]]", parts.join(","))
                    };
                    const TCL_NOISE: &[&str] = &[
                        "await", "yield", "new", "not", "typeof", "delete", "void", "throw", "raise", "async",
                        "lambda", "in", "is", "and", "or", "if", "else", "for", "of", "as", "then", "do",
                        "instanceof", "sizeof", "return", "my", "local", "let", "var", "const", "final",
                        "mut", "ref", "unsafe", "defer", "go", "our", "pub",
                    ];
                    if !is_flag && TCL_NOISE.contains(&clean.as_str()) {
                        // Reference lexer skips NOISE words and does not trigger juxtaposed calls on preceding words
                        word_forms.push((*wline, None, "0".to_string(), false));
                    } else if Self::tcl_is_ident(&clean) {
                        // A -flag starts with '-', which is Op('-') so it does NOT trigger a call on the preceding word
                        word_forms.push((*wline, Some(clean), seg_form, !is_flag));
                    } else {
                        let is_val = !is_flag && w.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
                        word_forms.push((*wline, None, seg_form, is_val));
                    }
                }
            }
        }
        // Emit x-events for continuation-line words (wline != cmd_line)
        let mut cont_line: u32 = 0;
        let mut cont_parts: Vec<String> = Vec::new();
        for &(wline, _, ref form_str, _) in &word_forms {
            if wline != cmd_line && form_str != "0" && !form_str.is_empty() {
                if cont_line != 0 && wline != cont_line && !cont_parts.is_empty() {
                    let v = if cont_parts.len() == 1 {
                        cont_parts.remove(0)
                    } else {
                        format!("[\"m\",[{}]]", cont_parts.join(","))
                    };
                    self.lexer_event(b'x', cont_line, &[], &v, false);
                    cont_parts.clear();
                }
                cont_line = wline;
                cont_parts.push(form_str.clone());
            }
        }
        if cont_line != 0 && !cont_parts.is_empty() {
            let v = if cont_parts.len() == 1 {
                cont_parts.remove(0)
            } else {
                format!("[\"m\",[{}]]", cont_parts.join(","))
            };
            self.lexer_event(b'x', cont_line, &[], &v, false);
        }
        // Right-nest command calls on the same line ONLY when followed by a value token (not { or [)
        let mut acc = String::new();
        for idx in (0..word_forms.len()).rev() {
            let (wline, ref ident_opt, ref form_str, _) = word_forms[idx];
            let next_is_val = idx + 1 < word_forms.len() && word_forms[idx + 1].3 && word_forms[idx + 1].0 == wline;
            if acc.is_empty() {
                acc = form_str.clone();
            } else if let Some(cname) = ident_opt {
                if next_is_val {
                    let cid = self.push_call(cname.clone(), wline);
                    acc = format!("[\"c\",{cid},0,[{acc}]]");
                } else {
                    let next_is_bracket = idx + 1 < word_forms.len()
                        && word_forms[idx + 1].0 == wline
                        && word_forms[idx + 1].2.starts_with("[\"t\",");
                    if next_is_bracket {
                        let _ = self.push_call(cname.clone(), wline);
                    }
                    acc = format!("[\"m\",[{form_str},{acc}]]");
                }
            } else {
                acc = format!("[\"m\",[{form_str},{acc}]]");
            }
        }
        acc
    }

    fn tcl_scan_script(&mut self, text: &str, start_line: u32, depth: usize) {
        if depth > 24 {
            return;
        }
        let cmds = Self::tcl_tokenize_commands(text, start_line);
        for (cline, words) in cmds {
            let ret_pos = words.iter().position(|w| w.1 == "return");
            if let Some(rp) = ret_pos {
                if rp == 0 {
                    let val = if words.len() > 1 {
                        self.tcl_eval_words(&words[1..], cline, depth)
                    } else {
                        "0".to_string()
                    };
                    let rv = if val.is_empty() { "0".to_string() } else { val };
                    self.lexer_event(b'r', cline, &[], &rv, false);
                } else {
                    let val = self.tcl_eval_words(&words, cline, depth);
                    if !val.is_empty() && val != "0" {
                        self.lexer_event(b'x', cline, &[], &val, false);
                    }
                    let rline = words[rp].0;
                    let rval = if rp + 1 < words.len() {
                        self.tcl_eval_words(&words[rp + 1..], rline, depth)
                    } else {
                        "0".to_string()
                    };
                    let rv = if rval.is_empty() { "0".to_string() } else { rval };
                    self.lexer_event(b'r', rline, &[], &rv, false);
                }
            } else {
                let val = self.tcl_eval_words(&words, cline, depth);
                if !val.is_empty() && val != "0" {
                    self.lexer_event(b'x', cline, &[], &val, false);
                }
            }
        }
    }

    fn tcl_proc_command(&mut self, id: u32) {
        let line = self.line(id);
        let children = self.children(id);
        let words: Vec<u32> = children
            .into_iter()
            .filter(|&c| !matches!(self.kind(c), "whitespace" | "comment" | "newline" | ";" | "command_end"))
            .collect();
        // words[0] is "proc", words[1] is name, last braced_word is body
        let raw_name = words.get(1).map(|&c| self.text_string(c)).unwrap_or_default();
        let sym_name = Self::tcl_clean_name(raw_name.trim_matches(|c| c == '{' || c == '}'));
        if sym_name.is_empty() {
            return;
        }
        let args_braced = words.get(2).is_some_and(|&c| self.kind(c) == "braced_word");
        let body_braced = words.len() >= 4 && words.last().is_some_and(|&c| self.kind(c) == "braced_word");
        if !args_braced || !body_braced {
            self.push_call("proc".to_string(), line);
        }
        let sym = self.add_symbol(sym_name, String::new(), KIND_FUNCTION, line, Vec::new());
        self.stack.push(Frame {
            node: id,
            symbol: sym,
            class: String::new(),
            receiver: String::new(),
            container: false,
            defines: Some(sym),
        });
        if let Some(&body_node) = words.last() {
            if words.len() >= 4 && self.kind(body_node) == "braced_word" {
                let raw_body = self.text_string(body_node);
                if raw_body.starts_with('{') && raw_body.ends_with('}') && raw_body.len() >= 2 {
                    self.tcl_scan_script(&raw_body[1..raw_body.len() - 1], self.line(body_node), 1);
                }
            } else {
                self.push_call("proc".to_string(), line);
            }
        }
        self.stack.pop();
    }

    fn tcl_call_command(&mut self, id: u32) {
        let line = self.line(id);
        let children = self.children(id);
        let mut words: Vec<(u32, String)> = Vec::new();
        for c in children {
            let ck = self.kind(c);
            if matches!(ck, "whitespace" | "comment" | "newline" | ";" | "command_end") {
                continue;
            }
            words.push((self.line(c), self.text_string(c)));
        }
        let is_ret = words.first().is_some_and(|w| w.1 == "return");
        let val = if is_ret && words.len() > 1 {
            self.tcl_eval_words(&words[1..], line, 0)
        } else {
            self.tcl_eval_words(&words, line, 0)
        };
        if is_ret {
            let rv = if val.is_empty() { "0".to_string() } else { val };
            self.lexer_event(b'r', line, &[], &rv, false);
        } else if !val.is_empty() && val != "0" {
            self.lexer_event(b'x', line, &[], &val, false);
        }
    }

    pub(crate) fn enter(&mut self, id: u32) {
        let kind = self.kind(id);
        if self.is_rust() && self.enter_rust(id) {
            return;
        }
        if self.facts.tag == "TCL" && kind == "proc_command" {
            self.tcl_proc_command(id);
            return;
        }
        // C, C++: the declared name lives inside a declarator, so the node
        // cannot be read off a `Def` table. `enter_c` only claims the nodes it
        // actually reads, and the generic walk keeps everything else.
        if self.is_c() && self.enter_c(id) {
            return;
        }
        // `public abstract T Create(Type objectType);` in an interface or an
        // abstract class has no body: the grammar calls it a declaration and
        // the lexer reads a call, so every abstract member of the corpus lost
        // one. Read before the definition table claims the node. An
        // expression-bodied method (`=>`) has a body and is read below instead.
        // `public delegate object ObjectConstructor<T>(params object[] args);`:
        // a delegate declaration is one header statement to the one-pass
        // reader, and the definition table claims the node before any arm can
        // read it, so every delegate declaration lost its line.
        if (kind == "method_declaration"
            && self.kinds.calls.contains(&"object_creation_expression")
            && !self
                .children(id)
                .into_iter()
                .any(|child| matches!(self.kind(child), "block" | "=>")))
            || (kind == "delegate_declaration" && self.language.name == "csharp")
        {
            let children = self.children(id);
            let named = children
                .iter()
                .copied()
                .take_while(|&child| !matches!(self.kind(child), "parameter_list" | "formal_parameters" | "type_parameters"))
                .filter(|&child| self.kind(child) == "identifier" || self.leaf_name(child).is_some())
                .last();
            if let Some(named) = named {
                let name = self.text_string(named);
                let call_line = self.line(named);
                let index = self.push_call(name, call_line);
                let mut line = self.line(id);
                if self.language.name == "csharp" {
                    if let Some(first) = children.iter().copied().find(|&child| {
                        matches!(self.kind(child), "attributes" | "attribute_section" | "attribute_list")
                    }) {
                        line = self.line(first);
                    }
                }
                let mut parts: Vec<String> = Vec::new();
                let mut attributed = false;
                // The header words run up to the parameter list, so the name and
                // the generic parameters (`MapFrom<TMember>`) reach the value the
                // way the lexer writes them; stopping at `type_parameters` lost
                // every generic abstract member's name and type parameters.
                for child in &children {
                    if matches!(self.kind(*child), "parameter_list" | "formal_parameters") {
                        break;
                    }
                    if self.language.name == "csharp"
                        && matches!(self.kind(*child), "attributes" | "attribute_section" | "attribute_list")
                    {
                        self.csharp_attributes(*child, &mut attributed, &mut parts);
                        continue;
                    }
                    for word in self.c_words_of(*child) {
                        let mut form = String::from("[\"n\",");
                        push_string(&mut form, &word);
                        form.push(']');
                        parts.push(form);
                    }
                }
                let mut args: Vec<String> = Vec::new();
                if let Some(list) = children
                    .iter()
                    .copied()
                    .find(|&child| matches!(self.kind(child), "parameter_list" | "formal_parameters"))
                {
                    for parameter in self.children(list) {
                        if self.kind(parameter) != "parameter" {
                            continue;
                        }
                        let mut p_parts: Vec<String> = Vec::new();
                        let mut p_attr = false;
                        for p_child in self.children(parameter) {
                            if self.language.name == "csharp"
                                && matches!(self.kind(p_child), "attributes" | "attribute_section" | "attribute_list")
                            {
                                self.csharp_attributes(p_child, &mut p_attr, &mut p_parts);
                                continue;
                            }
                            for word in self.c_words_of(p_child) {
                                let word = if word == "this" { "self" } else { word.as_str() };
                                let mut form = String::from("[\"n\",");
                                push_string(&mut form, word);
                                form.push(']');
                                p_parts.push(form);
                            }
                        }
                        if !p_parts.is_empty() {
                            args.push(format!("[\"m\",[{}]]", p_parts.join(",")));
                        }
                    }
                }
                let call = format!("[\"c\",{index},0,[{}]]", args.join(","));
                parts.push(call);
                for child in &children {
                    if self.kind(*child) != "type_parameter_constraints" {
                        continue;
                    }
                    for word in self.c_words_of(*child) {
                        let mut form = String::from("[\"n\",");
                        push_string(&mut form, &word);
                        form.push(']');
                        parts.push(form);
                    }
                }
                let value = format!("[\"m\",[{}]]", parts.join(","));
                self.lexer_event(b'x', line, &[], &value, false);
            }
        }
        if kind == "lib_definition" && self.facts.tag == "CR" {
            let line = self.line(id);
            let idx = self.push_call("lib".to_string(), line);
            let cname = self.children(id).into_iter().find(|&c| matches!(self.kind(c), "constant" | "scope")).map(|c| self.text_string(c)).unwrap_or_default();
            let val = if cname.is_empty() {
                format!("[\"c\",{idx},0,[]]")
            } else {
                let mut arg = String::from("[\"n\",");
                push_string(&mut arg, &cname);
                arg.push(']');
                format!("[\"c\",{idx},0,[{arg}]]")
            };
            self.lexer_event(b'x', line, &[], &val, false);
        }
        if kind == "fun_declaration" && self.facts.tag == "CR" {
            let line = self.line(id);
            let idx = self.push_call("fun".to_string(), line);
            let fname = self.children(id).into_iter().find(|&c| self.kind(c) == "identifier").map(|c| self.text_string(c)).unwrap_or_default();
            let val = if fname.is_empty() {
                format!("[\"c\",{idx},0,[]]")
            } else {
                let mut arg = String::from("[\"n\",");
                push_string(&mut arg, &fname);
                arg.push(']');
                format!("[\"c\",{idx},0,[{arg}]]")
            };
            self.lexer_event(b'x', line, &[], &val, false);
        }
        if let Some(def) = self.kinds.definitions.iter().find(|def| def.kind == kind) {
            self.open_definition(id, def);
            if self.language.name == "csharp" && !def.container {
                self.cs_arrow_body(id, self.line(id));
            }
            return;
        }
        if self.has_kind(id, self.kinds.imports) {
            self.import(id);
            return;
        }
        // Go and Java write a statement where Python writes an expression and
        // JavaScript a declarator; their arms are the lexer's statement model,
        // read off the tree, and none of them is any arm below.
        match self.kinds.family {
            Family::Go => {
                self.go_enter(id);
                return;
            }
            Family::Java => {
                self.java_enter(id);
                return;
            }
            Family::Plain | Family::Js | Family::Rust => {}
        }
        match kind {
            // A field declaration is a statement to the lexer: `private readonly
            // DeserializationTracker _tracker;` is an `x` event over the whole
            // header, and nothing read the node, so every C# field an API
            // declares left an event the tree never wrote. Go has the same arm
            // in its own statement model.
            "field_declaration" | "event_field_declaration" | "event_declaration" => {
                // A field initialised with a call is an assignment to the
                // lexer; anything else is the header event.
                if !self.cs_field_assignment(id) {
                    // `private int _mask = 31;`: a field with a constant
                    // initialiser is an assignment to 0 for the lexer, and the
                    // arm wrote the header alone - no event on the line at all.
                    let children = self.children(id);
                    let assigned = children.iter().position(|&child| self.kind(child) == "=");
                    let target = assigned.and_then(|at| self.cs_decl_target(&children[..at]));
                    match (assigned, target) {
                        (Some(at), Some(name)) => {
                            // C#: the initialiser is the value. `public bool Flag
                            // = false;` is `a [Flag] n:false` to the lexer, and
                            // writing the constant `0` for every field but a call
                            // left each such field a disagreement. A keyword
                            // literal is a name to the one-pass reader, and a
                            // constant's initialiser merges the name and the
                            // constant, so the written form is the child's own.
                            // Other languages keep the constant: the model is the
                            // C# arm's, measured here, not theirs.
                            let form = if self.language.name == "csharp" {
                                let child = children[at + 1..]
                                    .iter()
                                    .copied()
                                    .find(|&c| !self.is_layout(c) && self.kind(c) != "ref");
                                let kind = child.map(|node| self.kind(node)).unwrap_or("");
                                let first_form = match kind {
                                    "true" | "false" | "null" => format!("[\"n\",\"{kind}\"]"),
                                    "" => "0".to_string(),
                                    _ => child
                                        .map(|node| self.form(node))
                                        .filter(|form| !form.is_empty() && form != "0")
                                        .unwrap_or_else(|| "0".to_string()),
                                };
                                let mut multi = vec![first_form];
                                for (idx, &c) in children.iter().enumerate() {
                                    if idx > at && self.kind(c) == "=" {
                                        if let Some(&rhs) = children[idx + 1..]
                                            .iter()
                                            .find(|&&n| !self.is_layout(n) && self.kind(n) != "ref")
                                        {
                                            let rhs_form = self.form(rhs);
                                            if let Some(t_name) = children[..idx].iter().rev().find_map(|&n| self.leaf_name(n)) {
                                                let mut tf = String::from("[\"n\",");
                                                push_string(&mut tf, &t_name);
                                                tf.push(']');
                                                let rhs_val = if rhs_form.is_empty() { "0" } else { rhs_form.as_str() };
                                                multi.push(format!("[\"m\",[{tf},{rhs_val}]]"));
                                            }
                                        }
                                    }
                                }
                                if multi.len() == 1 {
                                    multi.remove(0)
                                } else {
                                    format!("[\"t\",[{}]]", multi.join(","))
                                }
                            } else {
                                "0".to_string()
                            };
                            let line = self.line(id);
                            self.lexer_event(b'a', line, &[name], &form, false);
                        }
                        _ => {
                            self.decl_statement(id);
                            self.field_initializer_calls(id);
                        }
                    }
                }
                self.cs_declared_vars(id);
                if self.language.name == "csharp" {
                    self.cs_lambda_symbol(id);
                }
            }
            // `uint A(int hc) => hc.ToHashCode();`: an expression-bodied member
            // writes its body after the arrow, and the one-pass reader drops
            // the member header before `=>` and records only that body.
            "expression_bodied_member" if self.language.name == "csharp" => {
                self.cs_arrow_body(id, self.line(id));
            }
            // `var o = JObject.Parse(text);` and `int depth = 0;` and `using var x = ...;`:
            // C# local and using declarations are assignment statements to the
            // lexer. `cs_field_assignment` handles direct calls/creations; any
            // other initialised declaration (literals, names, member reads,
            // expressions) still writes an `a` event, matching `field_declaration`.
            "local_variable_declaration" | "using_declaration"
                if self.kinds.calls.contains(&"object_creation_expression") =>
            {
                if !self.cs_field_assignment(id) {
                    let children = self.children(id);
                    let assigned = children.iter().position(|&child| self.kind(child) == "=");
                    let target = assigned.and_then(|at| self.cs_decl_target(&children[..at]));
                    match (assigned, target) {
                        (Some(at), Some(name)) => {
                            let child = children[at + 1..]
                                .iter()
                                .copied()
                                .find(|&c| !self.is_layout(c) && self.kind(c) != "ref");
                            let kind = child.map(|node| self.kind(node)).unwrap_or("");
                            let first_form = match kind {
                                "true" | "false" | "null" => format!("[\"n\",\"{kind}\"]"),
                                "" => "0".to_string(),
                                _ => child
                                    .map(|node| self.form(node))
                                    .filter(|form| !form.is_empty() && form != "0")
                                    .unwrap_or_else(|| "0".to_string()),
                            };
                            let mut multi = vec![first_form];
                            for (idx, &c) in children.iter().enumerate() {
                                if idx > at && self.kind(c) == "=" {
                                    if let Some(&rhs) = children[idx + 1..]
                                        .iter()
                                        .find(|&&n| !self.is_layout(n) && self.kind(n) != "ref")
                                    {
                                        let rhs_form = self.form(rhs);
                                        if let Some(t_name) = children[..idx].iter().rev().find_map(|&n| self.leaf_name(n)) {
                                            let mut tf = String::from("[\"n\",");
                                            push_string(&mut tf, &t_name);
                                            tf.push(']');
                                            if rhs_form == "0" || rhs_form.is_empty() {
                                                multi.push(tf);
                                            } else {
                                                multi.push(format!("[\"m\",[{tf},{rhs_form}]]"));
                                            }
                                        }
                                    }
                                }
                            }
                            let form = if multi.len() == 1 {
                                multi.remove(0)
                            } else {
                                format!("[\"t\",[{}]]", multi.join(","))
                            };
                            let line = self.line(id);
                            self.lexer_event(b'a', line, &[name], &form, false);
                        }
                        _ => {
                            self.decl_statement(id);
                            self.field_initializer_calls(id);
                        }
                    }
                }
                self.cs_declared_vars(id);
                if kind == "local_variable_declaration" {
                    self.cs_lambda_symbol(id);
                }
            }
            // `size_t n = strlen(s);`: a declaration with an initializer is one
            // statement to the lexer, and the call in it is a fact. C, C++ and
            // Objective-C declare this way on nearly every line, so nothing
            // read the node and every such call was lost.
            "declaration_statement" => self.decl_statement(id),
            // `val a = 1` in Kotlin: the lexer files an assignment to the
            // declared name, and nothing opened the property, so every
            // initialised property in a Kotlin class or object lost its line.
            // Swift spells it the same way (`let x = 1`), with the name as a
            // plain identifier instead of a `property_head`, so the arm reads
            // both.
            // C# spells a property `public int? Index { get; set; }`, which the
            // lexer reads as a declaration header and its accessors as their
            // own statements - not as the Kotlin/Swift assignment the property
            // arm writes. Dispatched by the grammar's own call kinds.
            // `public IJEnumerable<int> this[object key] { get; }`: an indexer
            // is a header statement to the lexer, with `this` spelled `self`.
            // `public static bool operator !=(V a, V b)` and conversion
            // operators are the same header statement, unless expression-bodied.
            "indexer_declaration" | "operator_declaration" | "conversion_operator_declaration" => {
                if !(self.language.name == "csharp" && self.cs_arrow_body(id, self.line(id))) {
                    self.java_header_event(id);
                }
            }
            "class_member" if self.facts.tag == "PHP" => {
                let ch = self.children(id);
                if let Some(md) = ch.iter().copied().find(|&c| self.kind(c) == "member_declaration") {
                    if let Some(pd) = self.child_of_kind(md, "property_declaration") {
                        let pchildren = self.children(pd);
                        let has_eq = pchildren.iter().any(|&c| self.kind(c) == "=");
                        let has_q = pchildren.iter().any(|&c| self.kind(c) == "?");
                        if !has_eq || has_q {
                            let line = self.line(id);
                            let mut parts = Vec::new();
                            for &c in &ch {
                                if self.kind(c) == "modifiers" {
                                    for mc in self.children(c) {
                                        let t = self.text_string(mc);
                                        if !t.is_empty() && t.bytes().next().is_some_and(|b| b.is_ascii_alphabetic()) {
                                            let mut n = String::from("[\"n\",");
                                            push_string(&mut n, &t);
                                            n.push(']');
                                            parts.push(n);
                                        }
                                    }
                                }
                            }
                            for &c in &pchildren {
                                let ck = self.kind(c);
                                if matches!(ck, ";" | "," | "=" | "?" | "|") {
                                    continue;
                                }
                                if matches!(ck, "variable" | "variable_name") {
                                    let nm = self.text_string(c).trim_start_matches('$').to_string();
                                    if !nm.is_empty() {
                                        let mut n = String::from("[\"n\",");
                                        push_string(&mut n, &nm);
                                        n.push(']');
                                        parts.push(n);
                                    }
                                } else if matches!(ck, "true" | "false" | "null") {
                                    parts.push(format!("[\"n\",\"{ck}\"]"));
                                } else if !self.is_layout(c) {
                                    let f = self.form(c);
                                    if !f.is_empty() && f != "0" {
                                        parts.push(f);
                                    } else {
                                        let t = self.text_string(c);
                                        if !t.is_empty() && t.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                                            let mut n = String::from("[\"n\",");
                                            push_string(&mut n, &t);
                                            n.push(']');
                                            parts.push(n);
                                        }
                                    }
                                } else {
                                    let t = self.text_string(c);
                                    if matches!(t.as_str(), "int" | "string" | "bool" | "float" | "array" | "object" | "mixed" | "callable" | "iterable" | "self" | "parent" | "static" | "void" | "never" | "null" | "true" | "false") {
                                        let mut n = String::from("[\"n\",");
                                        push_string(&mut n, &t);
                                        n.push(']');
                                        parts.push(n);
                                    }
                                }
                            }
                            if !parts.is_empty() {
                                let val = if parts.len() == 1 {
                                    parts.remove(0)
                                } else {
                                    format!("[\"m\",[{}]]", parts.join(","))
                                };
                                self.lexer_event(b'x', line, &[], &val, false);
                                self.consumed.push(pd);
                            }
                        }
                    }
                }
            }
            "property_declaration" | "const_declaration" if self.facts.tag == "PHP" => {
                if self.consumed.contains(&id) {
                    return;
                }
                let children = self.children(id);
                let line = self.line(id);
                if let Some(eq) = children.iter().position(|&c| self.kind(c) == "=") {
                    let target = children[..eq]
                        .iter()
                        .rev()
                        .copied()
                        .find(|&c| matches!(self.kind(c), "variable" | "variable_name" | "name" | "identifier"))
                        .map(|c| self.text_string(c).trim_start_matches('$').to_string())
                        .filter(|s| !s.is_empty());
                    if let Some(name) = target {
                        let rhs = children[eq + 1..].iter().copied().find(|&c| !self.is_layout(c));
                        let val = rhs
                            .map(|c| {
                                let k = self.kind(c);
                                if matches!(k, "true" | "false" | "null") {
                                    format!("[\"n\",\"{k}\"]")
                                } else {
                                    let f = self.form(c);
                                    if f.is_empty() { "0".to_string() } else { f }
                                }
                            })
                            .unwrap_or_else(|| "0".to_string());
                        self.lexer_event(b'a', line, &[name], &val, false);
                    } else {
                        self.decl_statement(id);
                    }
                } else {
                    self.decl_statement(id);
                }
            }
            "echo_statement" if self.facts.tag == "PHP" => {
                let line = self.line(id);
                let mut parts = vec!["[\"n\",\"echo\"]".to_string()];
                for c in self.children(id) {
                    let ck = self.kind(c);
                    if matches!(ck, "echo" | ";" | "," | "?>" | "<?=" ) {
                        continue;
                    }
                    if self.is_layout(c) && !matches!(ck, "string" | "encapsed_string" | "integer" | "float") {
                        continue;
                    }
                    let f = self.form(c);
                    parts.push(if f.is_empty() { "0".to_string() } else { f });
                }
                let val = if parts.len() == 1 {
                    parts.remove(0)
                } else {
                    format!("[\"m\",[{}]]", parts.join(","))
                };
                self.lexer_event(b'x', line, &[], &val, false);
            }
            "trait_adaptation" if self.facts.tag == "PHP" => {
                self.decl_statement(id);
            }
            "property_declaration" => {
                if self.kinds.calls.contains(&"object_creation_expression") {
                    if !(self.language.name == "csharp" && self.cs_arrow_body(id, self.line(id))) {
                        self.java_header_event(id);
                    }
                } else {
                    self.kt_property(id);
                }
                // `public bool VerifyParent { get; set; } = true;`: the tail
                // after the accessors is an initialiser the lexer reads as an
                // expression statement of its own, and nothing read it, so the
                // line carried a header event and no value (C# corpus: 9 files).
                if self.facts.tag == "CS" {
                    let tail = self.children(id).into_iter().find(|&child| self.kind(child) == "property_tail");
                    let rhs: Vec<u32> = tail
                        .map(|tail| self.children(tail))
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|&child| !self.is_layout(child) && self.kind(child) != "=")
                        .collect();
                    // `= new("All") { Patterns = ... }`: when the constructor and
                    // its object_initializer sit as siblings in `property_tail`,
                    // taking only the last child drops the constructor's
                    // arguments; merge them unless it is an empty `new()`.
                    let rhs_slice = if rhs.len() > 1
                        && self.kind(rhs[0]) == "implicit_creation"
                        && self.children(rhs[0]).into_iter().all(|c| {
                            self.is_layout(c)
                                || self.kind(c) == "new"
                                || (self.kind(c) == "argument_list"
                                    && !self.children(c).into_iter().any(|a| !self.is_layout(a)))
                        }) {
                        &rhs[1..]
                    } else {
                        &rhs[..]
                    };
                    if let Some(&first) = rhs_slice.first() {
                        let line = self.line(first);
                        let form = if rhs_slice.len() == 1 {
                            self.form(first)
                        } else {
                            self.merge_form(rhs_slice)
                        };
                        if form != "0" {
                            self.event(&format!("[\"x\",{line},{form}]"));
                        }
                    }
                }
            }
            // `rootProject.name = "demo"` in a .kts script: the grammar keeps
            // the line as one flat node, nothing read it, and every Gradle
            // script in the corpus lost its assignments (rootProject.name,
            // group, version: 165 rows).
            "script_statement" => self.kt_script_statement(id),
            // `package akka`: Scala's clause is the same event Kotlin's is, and
            // without it every file with a package statement lost its first
            // line.
            "package_clause" => self.kt_package(id),
            // `@throws(classOf[X])`, `@deprecated("old")`: the lexer reads an
            // annotation as a call of its name, and Scala's stdlib is covered
            // in them.
            // Swift writes its attributes with the same shape under another
            // name: `@inline(never)`, `@_spi(Formatting)`, `@_semantics(…)`.
            // The lexer reads each as a call of its name.
            // Swift and Scala write attributes with this node; Python's `attribute`
            // (a dotted name like `asyncio.open_connection`) is the same kind, so an
            // unguarded arm pushed a call named after the receiver - the python fixture
            // async_worker.py reported deep-only calls asyncio|7 and item|37.
            "attribute" if matches!(self.language.name, "swift" | "scala") => {
                if let Some(node) = self.children(id).into_iter().find(|&child| matches!(self.kind(child), "identifier")) {
                    let name = self.text_string(node);
                    let line = self.line(id);
                    let index = self.push_call(name, line);
                    // The line is an `x` event whose value is that call, with
                    // the attribute's own arguments - `c(0;[n:never])`.
                    if let Some(list) = self
                        .children(id)
                        .into_iter()
                        .find(|&child| matches!(self.kind(child), "attribute_arguments" | "annotation_arguments"))
                    {
                        if self.children(list).iter().any(|&child| !self.is_layout(child)) {
                            let args = self.arguments(list);
                            self.event(&format!("[\"x\",{line},[\"c\",{index},0,{args}]]"));
                        }
                    }
                }
            }
            // `[MarshalAs(UnmanagedType.LPStr)]`, `[TestCase(1, 2)]`: the
            // one-pass reader has no idea an attribute section is a
            // declaration, so it files the attribute's name as a call and the
            // CST, which reads the attribute as an attribute, had no call to
            // pair it with - the largest call cluster left in C# (MarshalAs
            // alone: 111 files). The name is the section's last bare
            // identifier, so a qualified `[A.B.C]` keeps C, which is the leaf
            // the comparison pairs on.
            "attribute" if self.language.name == "csharp" => {
                let name = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| self.kind(child) == "identifier")
                    .last()
                    .map(|child| self.text_string(child))
                    .unwrap_or_default();
                if !name.is_empty() {
                    let line = self.line(id);
                    // `[Obsolete] int F()` is written on the member's header,
                    // but the attribute is the class's statement, not the
                    // member's: the one-pass reader files it while the class is
                    // open, so the call belongs to the enclosing symbol. Pushed
                    // onto the member, it took the first index and every call
                    // the method itself writes shifted by one, which changed a
                    // method's events when its attributes changed.
                    let current = self.frame().symbol;
                    // The attribute decorates the declaration whose frame is
                    // already open, and the one-pass reader files it while the
                    // enclosing scope is open - so its call belongs one level up.
                    // Pushed onto the member it took the first index and every
                    // call the member itself writes shifted by the attribute's
                    // count.
                    let decorated = self.stack.last().is_some_and(|frame| {
                        matches!(
                            self.kind(frame.node),
                            "method_declaration"
                                | "constructor_declaration"
                                | "destructor_declaration"
                                | "property_declaration"
                                | "field_declaration"
                                | "event_declaration"
                                | "indexer_declaration"
                                | "delegate_declaration"
                                | "operator_declaration"
                                | "conversion_operator_declaration"
                                | "class_declaration"
                                | "struct_declaration"
                                | "interface_declaration"
                                | "enum_declaration"
                                | "record_declaration"
                        )
                    });
                    let target = if decorated && self.stack.len() > 1 {
                        self.stack[self.stack.len() - 2].symbol
                    } else {
                        current
                    };
                    self.facts.symbols[target].calls.push(Call { path: name, line });
                }
            }
            // `@file:JvmName("Test")` heads a file and states no value: the
            // one-pass reader reads a statement of annotations as layout
            // (`kotlin_annotation_alone` accepts `(`, `)`, `,` and strings but
            // no `=` and no nested call), so filing the call `JvmName` invents
            // one the lexer never has and the `kotlin_file_calls_agree` gate
            // fails. The walk visits the nested `annotation_item` on its own,
            // so it is consumed here and the arm below skips it.
            // Only the bare literal form: `@file:WasmCoroutineMode(isStack
            // SwitchingMode = true)` and `@file:DependsOn("…", options =
            // arrayOf(…))` carry an `=` or a nested call, fall through the
            // matcher, and the lexer DOES file a call for them — silencing
            // those cost four corpus files (kotlin 480 -> 484).
            "file_annotation" if self.kinds.family == Family::Plain => {
                let text = self.text(id);
                let nested_call = {
                    let mut pending = self.children(id);
                    let mut found = false;
                    while let Some(child) = pending.pop() {
                        if self.kind(child) == "call_expression" {
                            found = true;
                            break;
                        }
                        pending.extend(self.children(child));
                    }
                    found
                };
                if !nested_call && !text.contains(&b'=') {
                    let items: Vec<u32> = self
                        .children(id)
                        .into_iter()
                        .filter(|&child| matches!(self.kind(child), "annotation_item" | "annotation"))
                        .collect();
                    self.consumed.extend(items);
                }
            }
            "annotation" | "annotation_item" if self.kinds.family == Family::Plain => {
                if self.consumed.contains(&id) {
                    return;
                }
                // `@kotlin.jvm.JvmName("x")`: a qualified annotation is a run of
                // identifier children, and the first one alone is `kotlin`. The
                // lexer records the whole path as the call, so every qualified
                // annotation in the corpus lost its call (`call
                // kotlin.jvm.JvmName`, 18 rows of the kotlin dump).
                let mut parts: Vec<String> = Vec::new();
                for child in self.children(id) {
                    if self.is_layout(child) {
                        continue;
                    }
                    match self.kind(child) {
                        "identifier" | "simple_identifier" => parts.push(self.text_string(child)),
                        // `@get:Option(…)`, `@file:JvmName(…)`: the use-site
                        // target is a keyword leaf in front of the name, not
                        // part of it. Breaking on it wrote no call at all, so
                        // every `@get:`-targeted annotation lost the call the
                        // lexer files (20 files of the kotlin corpus).
                        "get" | "set" | "field" | "param" | "property" | "receiver" | "file" | "delegate" if parts.is_empty() => {}
                        "annotation_arguments" => break,
                        // `@kotlin.internal.RequireKotlin(…)`: the segment
                        // `internal` is also a Kotlin modifier, so it arrives
                        // as a keyword leaf of its own instead of an
                        // identifier. Breaking there wrote the call `kotlin`
                        // and lost the path the lexer files.
                        _ if self.children(child).is_empty() => parts.push(self.text_string(child)),
                        _ => break,
                    }
                }
                if !parts.is_empty() {
                    let name = parts.join(".");
                    // The event names the call the way the lexer writes any
                    // dotted call: the receiver in the path slot and the last
                    // segment as the callee — `@kotlin.jvm.JvmName("y")` is
                    // `c(n:kotlin.jvm;[0])`, not a call with an empty receiver.
                    let receiver = if parts.len() > 1 {
                        let mut text = "[\"n\",".to_string();
                        push_string(&mut text, &parts[..parts.len() - 1].join("."));
                        text.push(']');
                        text
                    } else {
                        "0".to_string()
                    };
                    let line = self.line(id);
                    let index = self.push_call(name.clone(), line);
                    // The lexer files the annotation's own arguments as an
                    // event holding the call's form, as it does for any other
                    // call written as a statement.
                    if let Some(list) = self.children(id).into_iter().find(|&child| self.kind(child) == "annotation_arguments") {
                        let args = self.arguments(list);
                        self.event(&format!("[\"x\",{line},[\"c\",{index},{receiver},{args}]]"));
                    } else if self.language.name == "scala" {
                        // `@tailrec`, `@InternalApi`, `@inline`: a bare Scala
                        // annotation states just its name. The lexer files an
                        // `x` event for it, and writing only the call above
                        // left every such line with no value the guard could
                        // pair (`n:tailrec` against no event).
                        let mut value = "[\"n\",".to_string();
                        push_string(&mut value, &name);
                        value.push(']');
                        self.event(&format!("[\"x\",{line},{value}]"));
                    }
                }
            }
            // `public string Key { get; set; }`: the lexer reads each accessor
            // as an expression statement and names it `get` / `set`. Nothing
            // read `accessor_declaration`, so a property lost one event per
            // accessor — the largest single C# cluster left.
            // `get;` / `set;`: the notifier token is the value — a keyword leaf
            // carries no expression child, so the arm writes the name itself.
            "accessor_declaration" => {
                if self.language.name == "csharp" && self.cs_arrow_body(id, self.line(id)) {
                    return;
                }
                let name = self
                    .children(id)
                    .into_iter()
                    .map(|child| self.kind(child).to_string())
                    .find(|kind| matches!(kind.as_str(), "get" | "set" | "init" | "add" | "remove"))
                    .or_else(|| {
                        self.children(id)
                            .first()
                            .map(|&child| self.kind(child).to_string())
                    })
                    .unwrap_or_default();
                if !name.is_empty() {
                    let line = self.line(id);
                    // `get; private set;`: the accessor's own modifier words
                    // are the first elements of the value, the keyword last.
                    let mut parts: Vec<String> = Vec::new();
                    for child in self.children(id) {
                        if self.kind(child) == name {
                            break;
                        }
                        for word in self.c_words_of(child) {
                            let mut form = String::from("[\"n\",");
                            push_string(&mut form, &word);
                            form.push(']');
                            parts.push(form);
                        }
                    }
                    let mut keyword = String::from("[\"n\",");
                    push_string(&mut keyword, &name);
                    keyword.push(']');
                    parts.push(keyword);
                    let value = if parts.len() == 1 {
                        parts.pop().unwrap()
                    } else {
                        format!("[\"m\",[{}]]", parts.join(","))
                    };
                    self.lexer_event(b'x', line, &[], &value, false);
                }
                // `get => GetItem(index);`: an expression-bodied accessor's
                // body is an expression statement to the lexer, and its calls
                // were never read (C# corpus: 20 files).
                if self.facts.tag == "CS" {
                    if let Some(body) = self.cs_expression_body(id) {
                        self.form(body);
                    }
                }
            }
            // `public string Key { get; set; }`: the header is a statement to
            // the lexer, and the accessors are its own statements.
            "property_declaration" => self.java_header_event(id),
            // `enum E { A = 1, B }`: every member is an assignment statement to
            // the lexer, and the tree wrote nothing for it, so an enum's own
            // lines were the first loss on most enum-bearing files.
            // `enum E { Binary = 0x00, Number = 1, … }`: the lexer unpacks the
            // member list exactly as it unpacks a declarator list - one
            // assignment to the first member whose value is a tuple of the
            // rest (`t:[0,n:Number,m:[n:Text,0]]`) - so one assignment per
            // member disagreed on every enum in the corpus.
            "enum_body" => {
                let members: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| self.kind(child) == "enum_member")
                    .collect();
                self.consumed.extend(members.iter().copied());
                if let Some((first, rest)) = members.split_first() {
                    let line = self.line(*first);
                    let name = self.first_identifier(*first);
                    if let Some(name) = name {
                        let first_value = self.enum_member_value(*first);
                        let parts: Vec<String> = if rest.is_empty() {
                            vec![first_value]
                        } else {
                            let mut parts = vec![first_value];
                            for member in rest {
                                let attr_forms: Vec<String> = if self.language.name == "csharp" {
                                    let mut forms = Vec::new();
                                    let mut attributed = false;
                                    self.csharp_attributes(*member, &mut attributed, &mut forms);
                                    forms
                                } else {
                                    Vec::new()
                                };
                                let member_name = if self.language.name == "csharp" {
                                    self.children(*member)
                                        .into_iter()
                                        .find(|&c| self.kind(c) == "identifier")
                                        .map(|c| self.text_string(c))
                                        .or_else(|| self.first_identifier(*member))
                                } else {
                                    self.first_identifier(*member)
                                };
                                match (member_name, self.enum_member_value(*member)) {
                                    (Some(member_name), value) if value == "0" => {
                                        let mut form = String::from("[\"n\",");
                                        push_string(&mut form, &member_name);
                                        form.push(']');
                                        if attr_forms.is_empty() {
                                            parts.push(form);
                                        } else {
                                            let mut items = attr_forms;
                                            items.push(form);
                                            parts.push(format!("[\"m\",[{}]]", items.join(",")));
                                        }
                                    }
                                    (Some(member_name), value) => {
                                        let mut key = String::from("[\"n\",");
                                        push_string(&mut key, &member_name);
                                        key.push(']');
                                        let mut items = attr_forms;
                                        items.push(key);
                                        items.push(value);
                                        parts.push(format!("[\"m\",[{}]]", items.join(",")));
                                    }
                                    _ => {}
                                }
                            }
                            parts
                        };
                        let value = if parts.len() == 1 {
                            parts[0].clone()
                        } else {
                            format!("[\"t\",[{}]]", parts.join(","))
                        };
                        self.lexer_event(b'a', line, &[name], &value, false);
                    }
                }
            }
            "enum_member" => {
                if self.consumed.contains(&id) {
                    return;
                }
                let line = self.line(id);
                let name = self.first_identifier(id);
                if let Some(name) = name {
                    let value = self
                        .children(id)
                        .into_iter()
                        .filter(|&child| child != id)
                        .rev()
                        .find(|&child| {
                            !matches!(self.kind(child), "identifier" | "=" | ",")
                                && !self.is_layout(child)
                        })
                        .map(|child| self.decl_value(child))
                        .unwrap_or_else(|| "0".to_string());
                    let value = if value.is_empty() { "0".to_string() } else { value };
                    let targets = vec![name];
                    self.lexer_event(b'a', line, &targets, &value, false);
                }
            }
            // `return (byte[])_obj.GetValue(value, n);`, `(DataTable)conv.
            // ReadJson(...)`: the call sits inside a cast, and nothing
            // descended into that node - every cast-and-call in the C# corpus
            // lost its call.
            "cast_expression" => {
                let mut calls = Vec::new();
                for child in self.children(id) {
                    self.c_calls_in_pub(child, &mut calls);
                }
                for call in calls {
                    self.form(call);
                }
            }
            // `Assert.Throws(null, () => G(1))` and `Export(def, () => part.Get())`:
            // a lambda passed as an argument is the whole content of that
            // argument, and the expression reader gives a lambda no form, so
            // every call written inside one stayed in the tree and out of the
            // call table. They are calls all the same — the cast arm below has
            // the same shape of gap — so they are read here, and only the call
            // table is written: a lambda has no value of its own to compare.
            "lambda_expression" if self.language.name == "csharp" => {
                self.cs_lambda_assignment_symbol(id);
                let mut calls = Vec::new();
                self.lambda_calls(id, &mut calls);
                for call in calls {
                    self.form(call);
                }
            }
            // `new CVariation(t) { Attribute = new Variation("x") { Pri = 0 } }`:
            // a creation inside an object initialiser is a call of its own, and
            // the initialiser was read for its assignments alone, so every nested
            // creation stayed in the tree and out of the call table.
            // `yield return v;` hands `v` out of the body exactly as `return v;`
            // does, so it is read by the return handler. It used to fall into the
            // arm below, which forms only lambdas, so the yielded expression and
            // every call in it (and every `new T[]` event) vanished from the
            // CST: an iterator accessor such as `get { yield return f(); }` lost
            // its whole body.
            "yield_statement" if self.language.name == "csharp" => self.return_statement(id),
            "object_initializer" | "array_initializer" | "anonymous_body"
            | "using_statement"
            | "fixed_statement"
                if self.language.name == "csharp" =>
            {
                if matches!(kind, "object_initializer" | "array_initializer" | "anonymous_body") {
                    self.cs_initializer_vars(id);
                }
                if kind == "object_initializer" {
                    self.cs_initializer_first_member(id);
                }
                // `using (a.B()) stmt`: the C# grammar expects a declaration
                // inside `using (...)`, so an expression resource puts `(` into
                // an `ERROR` node and leaves `a.B` outside it as a type/name.
                // Recover calls from the flattened header tokens when errored.
                if kind == "using_statement" && self.cs_has_error(id) {
                    let mut leaves = Vec::new();
                    for child in self.children(id) {
                        if self.kind(child) != "using" {
                            self.cs_collect_leaves(child, &mut leaves);
                        }
                    }
                    if let Some(open) = leaves.iter().position(|&n| self.kind(n) == "(") {
                        let mut depth = 0i32;
                        let mut close = leaves.len();
                        for (i, &n) in leaves[open..].iter().enumerate() {
                            match self.kind(n) {
                                "(" => depth += 1,
                                ")" => {
                                    depth -= 1;
                                    if depth == 0 {
                                        close = open + i;
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        if close > open + 1 {
                            self.cs_recovered_calls_tokens(&leaves[open + 1..close]);
                        }
                    }
                }
                let mut calls = Vec::new();
                self.lambda_calls(id, &mut calls);
                for call in calls {
                    self.form(call);
                }
            }
            "ERROR" if self.language.name == "csharp" => {
                self.cs_recovered_calls(id);
                self.cs_recovered_events(id);
            }
            "expression_statement" if self.facts.tag == "LUA" => {
                let children = self.children(id);
                if let Some(atail) = children.iter().copied().find(|&c| self.kind(c) == "assignment_tail") {
                    let line = self.line(id);
                    let targets: Vec<String> = children
                        .iter()
                        .copied()
                        .take_while(|&c| c != atail)
                        .filter(|&c| !self.is_layout(c) && self.kind(c) != ",")
                        .map(|n| {
                            let p = self.path_of(n);
                            if p.is_empty() { self.text_string(n) } else { p }
                        })
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !targets.is_empty() {
                        let tname = targets[0].clone();
                        let raw_tail = self.text_string(atail);
                        if let Some((_, rhs)) = raw_tail.split_once('=') {
                            let rhs_t = rhs.trim();
                            if rhs_t.starts_with("require") {
                                let bytes = rhs_t.as_bytes();
                                let mut i = 0;
                                while i < bytes.len() {
                                    if bytes[i] == b'"' || bytes[i] == b'\'' {
                                        let q = bytes[i];
                                        i += 1;
                                        let mut val = String::new();
                                        while i < bytes.len() && bytes[i] != q {
                                            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                                                i += 1;
                                            }
                                            val.push(bytes[i] as char);
                                            i += 1;
                                        }
                                        let alias = tname.rsplit(|c| c == '.' || c == ':').next().unwrap_or("").to_string();
                                        if !val.is_empty() && !alias.is_empty() {
                                            self.facts.imports.push(Import {
                                                line,
                                                module: val,
                                                alias,
                                                kind: IMPORT_MODULE,
                                                ..Import::default()
                                            });
                                        }
                                        break;
                                    }
                                    i += 1;
                                }
                            }
                        }
                        let mut vals: Vec<String> = Vec::new();
                        if let Some(el) = self.children(atail).into_iter().find(|&c| self.kind(c) == "expression_list") {
                            for ec in self.children(el) {
                                let eck = self.kind(ec);
                                if matches!(eck, "nil" | "true" | "false") {
                                    vals.push(format!("[\"n\",\"{eck}\"]"));
                                    continue;
                                }
                                if self.is_layout(ec) || eck == "," {
                                    continue;
                                }
                                if eck == "function_expression" {
                                    let lhs_is_index = children
                                        .iter()
                                        .copied()
                                        .find(|&c| !self.is_layout(c) && c != atail)
                                        .is_some_and(|n| self.kind(n) == "index");
                                    if !lhs_is_index {
                                        let sym = tname.rsplit(|c| c == '.' || c == ':').next().unwrap_or(&tname).to_string();
                                        if !sym.is_empty() {
                                            self.bound.push((ec, sym));
                                        }
                                    }
                                }
                                let f = self.form(ec);
                                vals.push(if f.is_empty() { "0".to_string() } else { f });
                            }
                        }
                        let val = if vals.is_empty() {
                            "0".to_string()
                        } else if vals.len() == 1 {
                            vals.remove(0)
                        } else {
                            format!("[\"t\",[{}]]", vals.join(","))
                        };
                        self.lexer_event(b'a', line, &targets, &val, false);
                    }
                } else {
                    self.expression_statement(id);
                }
            }
            "expression_statement" => self.expression_statement(id),
            "command_call" if matches!(self.facts.tag.as_str(), "RB" | "CR") => {
                let val = self.form(id);
                if !val.is_empty() && val != "0" {
                    self.lexer_event(b'x', self.line(id), &[], &val, false);
                }
            }
            "if_statement" | "unless_statement" | "elsif_clause" | "case_statement" | "when_clause" | "while_statement" | "until_statement"
                if self.facts.tag == "RB" =>
            {
                let line = self.line(id);
                let mut parts = Vec::new();
                for ch in self.children(id) {
                    let ck = self.kind(ch);
                    if matches!(ck, "body" | "else_clause" | "elsif_clause" | "when_clause" | "then_token" | "end") {
                        break;
                    }
                    if matches!(ck, "if" | "unless" | "elsif" | "case" | "when" | "while" | "until") {
                        continue;
                    }
                    if self.is_layout(ch) {
                        continue;
                    }
                    let f = self.form(ch);
                    if !f.is_empty() && f != "0" {
                        parts.push(f);
                    }
                }
                if parts.len() == 1 {
                    self.lexer_event(b'x', line, &[], &parts[0], false);
                } else if parts.len() > 1 {
                    self.lexer_event(b'x', line, &[], &format!("[\"m\",[{}]]", parts.join(",")), false);
                }
            }
            "multiple_assignment" if self.facts.tag == "RB" => {
                let children = self.children(id);
                let line = self.line(id);
                if let Some(eq) = children.iter().position(|&c| self.kind(c) == "=") {
                    let mut targets = Vec::new();
                    for &c in &children[..eq] {
                        if self.is_layout(c) || self.kind(c) == "," {
                            continue;
                        }
                        let p = self.path_of(c);
                        if !p.is_empty() {
                            targets.push(p);
                        }
                    }
                    let mut vals = Vec::new();
                    for &c in &children[eq + 1..] {
                        if self.is_layout(c) || self.kind(c) == "," {
                            continue;
                        }
                        let f = self.form(c);
                        vals.push(if f.is_empty() { "0".to_string() } else { f });
                    }
                    if !targets.is_empty() && !vals.is_empty() {
                        let val = if vals.len() == 1 {
                            vals.remove(0)
                        } else {
                            format!("[\"t\",[{}]]", vals.join(","))
                        };
                        self.lexer_event(b'a', line, &targets, &val, false);
                    }
                }
            }
            "include_statement" if matches!(self.facts.tag.as_str(), "RB" | "CR") => {
                let line = self.line(id);
                let children = self.children(id);
                let kw = children
                    .first()
                    .map(|&c| self.text_string(c))
                    .filter(|k| matches!(k.as_str(), "include" | "extend" | "prepend"))
                    .unwrap_or_else(|| "include".to_string());
                let idx = self.push_call(kw, line);
                let mut args = Vec::new();
                for &c in children.iter().skip(1) {
                    if !self.is_layout(c) {
                        let v = self.form(c);
                        if !v.is_empty() {
                            args.push(v);
                        }
                    }
                }
                let val = format!("[\"c\",{idx},0,[{}]]", args.join(","));
                self.lexer_event(b'x', line, &[], &val, false);
            }
            "pragma_directive" if self.facts.tag == "SOL" => {
                let raw = self.text_string(id);
                if raw.contains('=') {
                    let line = self.line(id);
                    self.lexer_event(b'a', line, &["solidity".to_string()], "0", false);
                } else {
                    self.decl_statement(id);
                }
            }
            "alias_declaration" if self.facts.tag == "CR" => {
                let children = self.children(id);
                if let Some(eq) = children.iter().position(|&c| self.kind(c) == "=") {
                    if let Some(name) = children[..eq].iter().rev().find_map(|&c| self.leaf_name(c)) {
                        let rhs = children[eq + 1..].iter().copied().find(|&c| !self.is_layout(c));
                        let val = rhs.map(|c| {
                            let f = self.form(c);
                            if !f.is_empty() && f != "0" {
                                f
                            } else {
                                let t = self.text_string(c).trim().replace("::", ".");
                                if t.is_empty() {
                                    "0".to_string()
                                } else {
                                    let mut o = String::from("[\"n\",");
                                    push_string(&mut o, &t);
                                    o.push(']');
                                    o
                                }
                            }
                        }).unwrap_or_else(|| "0".to_string());
                        self.lexer_event(b'a', self.line(id), &[name], &val, false);
                    }
                }
            }
            "var_declaration" if self.facts.tag == "ZIG" => {
                let children = self.children(id);
                if let Some(eq) = children.iter().position(|&c| self.kind(c) == "=") {
                    if let Some(name) = children[..eq].iter().rev().find_map(|&c| self.leaf_name(c)) {
                        let rhs = children[eq + 1..].iter().copied().find(|&c| !self.is_layout(c));
                        let val = rhs.map(|c| self.form(c)).filter(|v| !v.is_empty()).unwrap_or_else(|| "0".to_string());
                        self.lexer_event(b'a', self.line(id), &[name], &val, false);
                    }
                }
            }
            "begin_block" if self.facts.tag == "AWK" => {
                self.lexer_event(b'x', self.line(id), &[], "[\"n\",\"BEGIN\"]", false);
            }
            "end_block" if self.facts.tag == "AWK" => {
                self.lexer_event(b'x', self.line(id), &[], "[\"n\",\"END\"]", false);
            }
            "print_statement" | "delete_statement" if self.facts.tag == "AWK" => {
                self.decl_statement(id);
            }
            "pattern_rule" if self.facts.tag == "AWK" => {
                if let Some(first) = self.children(id).into_iter().find(|&c| !self.is_layout(c) && self.kind(c) != "block") {
                    let val = self.form(first);
                    if !val.is_empty() && val != "0" {
                        self.lexer_event(b'x', self.line(first), &[], &val, false);
                    }
                }
            }
            "unit_header" | "program_header" if self.facts.tag == "PAS" => {
                let line = self.line(id);
                let kw = if kind == "unit_header" { "unit" } else { "program" };
                let name = self
                    .children(id)
                    .into_iter()
                    .find(|&c| matches!(self.kind(c), "dotted_name" | "identifier"))
                    .map(|c| self.text_string(c).to_ascii_lowercase())
                    .unwrap_or_default();
                let val = if name.is_empty() {
                    format!("[\"n\",\"{kw}\"]")
                } else {
                    let mut n = String::from("[\"n\",");
                    push_string(&mut n, &name);
                    n.push(']');
                    format!("[\"m\",[[\"n\",\"{kw}\"],{n}]]")
                };
                self.lexer_event(b'x', line, &[], &val, false);
            }
            "const_declaration" | "type_declaration" if self.facts.tag == "PAS" => {
                let children = self.children(id);
                let line = self.line(id);
                if let Some(eq) = children.iter().position(|&c| self.kind(c) == "=") {
                    if let Some(name) = children[..eq]
                        .iter()
                        .copied()
                        .find(|&c| !self.is_layout(c))
                        .map(|c| self.text_string(c).to_ascii_lowercase())
                        .filter(|n| !n.is_empty())
                    {
                        let mut parts = Vec::new();
                        for &c in &children[eq + 1..] {
                            let ck = self.kind(c);
                            if ck == "record" {
                                parts.push("[\"n\",\"record\"]".to_string());
                                continue;
                            }
                            if ck == "field_list" {
                                for fc in self.children(c) {
                                    let fck = self.kind(fc);
                                    if fck == ";" { break; }
                                    if self.is_layout(fc) || matches!(fck, ":" | ",") { continue; }
                                    if fck == "identifier_list" {
                                        for ic in self.children(fc) {
                                            if self.kind(ic) == "identifier" {
                                                let nm = self.text_string(ic).to_ascii_lowercase();
                                                let mut n = String::from("[\"n\",");
                                                push_string(&mut n, &nm);
                                                n.push(']');
                                                parts.push(n);
                                            }
                                        }
                                    } else {
                                        let f = self.form(fc);
                                        if !f.is_empty() && f != "0" {
                                            parts.push(f);
                                        }
                                    }
                                }
                                continue;
                            }
                            if matches!(ck, "nil" | "true" | "false") {
                                parts.push(format!("[\"n\",\"{ck}\"]"));
                            } else if !self.is_layout(c) && ck != ";" {
                                let f = self.form(c);
                                if !f.is_empty() && f != "0" {
                                    parts.push(f);
                                }
                            }
                        }
                        let val = if parts.is_empty() {
                            "0".to_string()
                        } else if parts.len() == 1 {
                            parts.into_iter().next().unwrap()
                        } else {
                            format!("[\"m\",[{}]]", parts.join(","))
                        };
                        self.lexer_event(b'a', line, &[name], &val, false);
                        return;
                    }
                }
                self.decl_statement(id);
            }
            "if_statement" | "while_statement" | "repeat_statement" | "case_statement" | "with_statement" if self.facts.tag == "PAS" => {
                let children = self.children(id);
                if kind == "repeat_statement" {
                    if let Some(until_pos) = children.iter().position(|&c| self.kind(c) == "until") {
                        if let Some(&cond) = children[until_pos + 1..].iter().find(|&&c| !self.is_layout(c)) {
                            let val = self.form(cond);
                            if !val.is_empty() && val != "0" {
                                self.lexer_event(b'x', self.line(cond), &[], &val, false);
                            }
                        }
                    }
                } else {
                    for c in children {
                        let ck = self.kind(c);
                        if self.is_layout(c) || matches!(ck, "if" | "while" | "case" | "with") {
                            continue;
                        }
                        if matches!(ck, "then" | "do" | "of") {
                            break;
                        }
                        let cond_val = self.form(c);
                        if !cond_val.is_empty() && cond_val != "0" {
                            let mut items = Vec::new();
                            if kind == "while_statement" {
                                items.push("[\"n\",\"while\"]".to_string());
                            }
                            items.push(cond_val);
                            if kind == "if_statement" || kind == "while_statement" {
                                let ch_all = self.children(id);
                                let kw_sep = if kind == "if_statement" { "then" } else { "do" };
                                if let Some(sp) = ch_all.iter().position(|&x| self.kind(x) == kw_sep) {
                                    if let Some(&br) = ch_all[sp + 1..].iter().find(|&&x| !self.is_layout(x)) {
                                        let is_blk = self.kind(br) == "block";
                                        let first_st = if is_blk {
                                            self.children(br).into_iter().find(|&x| {
                                                let xk = self.kind(x);
                                                !self.is_layout(x) && !matches!(xk, "begin" | "end" | ";")
                                            })
                                        } else {
                                            Some(br)
                                        };
                                        if let Some(st) = first_st {
                                            let is_assign = self.kind(st) == "assignment"
                                                || (self.kind(st) == "expression_statement"
                                                    && self.children(st).into_iter().any(|x| self.kind(x) == "assignment"));
                                            if !is_assign {
                                                let sf = self.form(st);
                                                if !sf.is_empty() && sf != "0" {
                                                    if is_blk {
                                                        items.push("[\"n\",\"begin\"]".to_string());
                                                    }
                                                    items.push(sf);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            let val = if items.len() == 1 {
                                items.into_iter().next().unwrap()
                            } else {
                                format!("[\"m\",[{}]]", items.join(","))
                            };
                            self.lexer_event(b'x', self.line(id), &[], &val, false);
                        }
                        break;
                    }
                    if kind == "if_statement" {
                        let ch2 = self.children(id);
                        let mut anchors: Vec<(u32, u32)> = Vec::new();
                        if let Some(tp) = ch2.iter().position(|&c| self.kind(c) == "then") {
                            if let Some(&br) = ch2[tp + 1..].iter().find(|&&c| !self.is_layout(c)) {
                                anchors.push((self.line(id), br));
                            }
                        }
                        if let Some(ep) = ch2.iter().position(|&c| self.kind(c) == "else") {
                            if let Some(&br) = ch2[ep + 1..].iter().find(|&&c| !self.is_layout(c)) {
                                anchors.push((self.line(ch2[ep]), br));
                            }
                        }
                        for (aline, br) in anchors {
                            let first_stmt = if self.kind(br) == "block" {
                                self.children(br).into_iter().find(|&c| {
                                    let ck = self.kind(c);
                                    !self.is_layout(c) && !matches!(ck, "begin" | "end" | ";")
                                })
                            } else {
                                Some(br)
                            };
                            if let Some(st) = first_stmt {
                                let assign_opt = if self.kind(st) == "expression_statement" {
                                    self.children(st).into_iter().find(|&c| self.kind(c) == "assignment")
                                } else if self.kind(st) == "assignment" {
                                    Some(st)
                                } else {
                                    None
                                };
                                if let Some(asg) = assign_opt {
                                    let is_then_near = aline == self.line(id) && self.line(asg) <= aline + 1;
                                    if self.line(asg) > aline + 1 || is_then_near {
                                        let ach = self.children(asg);
                                        if let Some(ap) = ach.iter().position(|&c| self.kind(c) == ":=") {
                                            if let Some(&lhs) = ach[..ap].iter().find(|&&c| !self.is_layout(c)) {
                                                let tname = self.path_of(lhs).to_ascii_lowercase();
                                                let tname = if tname.is_empty() {
                                                    self.text_string(lhs).to_ascii_lowercase()
                                                } else {
                                                    tname
                                                };
                                                if !tname.is_empty() {
                                                    let val = ach[ap + 1..]
                                                        .iter()
                                                        .copied()
                                                        .find(|&c| !self.is_layout(c))
                                                        .map(|c| {
                                                            let f = self.form(c);
                                                            if f.is_empty() { "0".to_string() } else { f }
                                                        })
                                                        .unwrap_or_else(|| "0".to_string());
                                                    let final_val = if is_then_near {
                                                        let cform = ch2.iter().copied()
                                                            .find(|&x| !self.is_layout(x) && !matches!(self.kind(x), "if" | "then" | "else"))
                                                            .map(|x| self.form(x))
                                                            .unwrap_or_default();
                                                        if !cform.is_empty() && cform != "0" {
                                                            if val == "0" { cform } else { format!("[\"m\",[{}]]", format!("{cform},{val}")) }
                                                        } else {
                                                            val
                                                        }
                                                    } else {
                                                        val
                                                    };
                                                    self.lexer_event(b'a', aline, &[tname], &final_val, false);
                                                }
                                            }
                                        }
                                    }
                                } else if self.kind(br) == "block" && self.line(st) > aline + 1 {
                                    let sf = self.form(st);
                                    if !sf.is_empty() && sf != "0" {
                                        let val = format!("[\"m\",[[\"n\",\"begin\"],{sf}]]");
                                        self.lexer_event(b'x', aline, &[], &val, false);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "for_statement" if self.facts.tag == "PAS" => {
                let children = self.children(id);
                let line = self.line(id);
                if let Some(assign_pos) = children.iter().position(|&c| self.kind(c) == ":=") {
                    if let Some(name) = children[..assign_pos]
                        .iter()
                        .copied()
                        .find(|&c| self.kind(c) == "identifier")
                        .map(|c| self.text_string(c).to_ascii_lowercase())
                    {
                        let mut parts = Vec::new();
                        for &c in &children[assign_pos + 1..] {
                            let ck = self.kind(c);
                            if ck == "do" { break; }
                            if matches!(ck, "to" | "downto") {
                                parts.push(format!("[\"n\",\"{ck}\"]"));
                                continue;
                            }
                            if self.is_layout(c) { continue; }
                            let f = self.form(c);
                            if !f.is_empty() && f != "0" {
                                parts.push(f);
                            }
                        }
                        let val = if parts.is_empty() {
                            "0".to_string()
                        } else if parts.len() == 1 {
                            parts.into_iter().next().unwrap()
                        } else {
                            format!("[\"m\",[{}]]", parts.join(","))
                        };
                        self.lexer_event(b'a', line, &[name], &val, false);
                    }
                }
            }
            "section_marker" if self.facts.tag == "PAS" => {
                let sec = self.text_string(id).to_ascii_lowercase();
                if sec == "implementation" || sec == "interface" {
                    let mut sib = self.node(id).next_sibling;
                    while sib != NONE && self.is_layout(sib) {
                        sib = self.node(sib).next_sibling;
                    }
                    if sib != NONE {
                        let line = self.line(id);
                        match self.kind(sib) {
                            "uses_clause" => {
                                let units: Vec<String> = self
                                    .children(sib)
                                    .into_iter()
                                    .filter(|&c| self.kind(c) == "dotted_name")
                                    .map(|c| self.text_string(c).to_ascii_lowercase())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                                if !units.is_empty() {
                                    let mut head = format!(r#"["m",[["n","{sec}"],["n","uses"],["n","#);
                                    push_string(&mut head, &units[0]);
                                    head.push_str("]]]");
                                    let val = if units.len() == 1 {
                                        head
                                    } else {
                                        let mut items = vec![head];
                                        for u in &units[1..] {
                                            let mut n = String::from(r#"["n","#);
                                            push_string(&mut n, u);
                                            n.push(']');
                                            items.push(n);
                                        }
                                        format!(r#"["t",[{}]]"#, items.join(","))
                                    };
                                    self.lexer_event(b'x', line, &[], &val, false);
                                }
                            }
                            "unit_end" if sec == "implementation" => {
                                self.lexer_event(b'x', line, &[], "[\"m\",[[\"n\",\"implementation\"],[\"n\",\"end\"]]]", false);
                            }
                            "routine_declaration" if sec == "implementation" => {
                                let rch = self.children(sib);
                                let kw = rch.first().map(|&c| self.text_string(c).to_ascii_lowercase()).unwrap_or_default();
                                let rname = rch.iter().copied().find(|&c| self.kind(c) == "identifier").map(|c| self.text_string(c).to_ascii_lowercase()).unwrap_or_default();
                                if !kw.is_empty() && !rname.is_empty() {
                                    let mut items = vec!["[\"n\",\"implementation\"]".to_string()];
                                    let mut n1 = String::from("[\"n\",");
                                    push_string(&mut n1, &kw);
                                    n1.push(']');
                                    items.push(n1);
                                    let mut n2 = String::from("[\"n\",");
                                    push_string(&mut n2, &rname);
                                    n2.push(']');
                                    items.push(n2);
                                    if let Some(col) = rch.iter().position(|&c| self.kind(c) == ":") {
                                        for &tc in &rch[col + 1..] {
                                            let tck = self.kind(tc);
                                            if tck == ";" { break; }
                                            if !self.is_layout(tc) {
                                                let tf = self.form(tc);
                                                if !tf.is_empty() && tf != "0" {
                                                    items.push(tf);
                                                }
                                            }
                                        }
                                    }
                                    self.lexer_event(b'x', line, &[], &format!("[\"m\",[{}]]", items.join(",")), false);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            "field_list" if self.facts.tag == "PAS" => {
                let children = self.children(id);
                let mut cur_line: Option<u32> = None;
                let mut cur_parts: Vec<String> = Vec::new();
                for c in children {
                    let ck = self.kind(c);
                    if ck == ";" {
                        if let Some(ln) = cur_line.take() {
                            if !cur_parts.is_empty() {
                                let val = if cur_parts.len() == 1 {
                                    cur_parts.remove(0)
                                } else {
                                    format!("[\"m\",[{}]]", cur_parts.join(","))
                                };
                                self.lexer_event(b'x', ln, &[], &val, false);
                                cur_parts.clear();
                            }
                        }
                        continue;
                    }
                    if self.is_layout(c) || matches!(ck, ":" | ",") {
                        continue;
                    }
                    if cur_line.is_none() {
                        cur_line = Some(self.line(c));
                    }
                    if ck == "identifier_list" {
                        for id_c in self.children(c) {
                            if self.kind(id_c) == "identifier" {
                                let nm = self.text_string(id_c).to_ascii_lowercase();
                                let mut n = String::from("[\"n\",");
                                push_string(&mut n, &nm);
                                n.push(']');
                                cur_parts.push(n);
                            }
                        }
                    } else {
                        let f = self.form(c);
                        if !f.is_empty() && f != "0" {
                            cur_parts.push(f);
                        } else if ck == "identifier" {
                            let nm = self.text_string(c).to_ascii_lowercase();
                            let mut n = String::from("[\"n\",");
                            push_string(&mut n, &nm);
                            n.push(']');
                            cur_parts.push(n);
                        }
                    }
                }
                if let Some(ln) = cur_line {
                    if !cur_parts.is_empty() {
                        let val = if cur_parts.len() == 1 {
                            cur_parts.remove(0)
                        } else {
                            format!("[\"m\",[{}]]", cur_parts.join(","))
                        };
                        self.lexer_event(b'x', ln, &[], &val, false);
                    }
                }
            }
            "variable_declaration" if self.facts.tag == "PAS" => {
                let children = self.children(id);
                let line = self.line(id);
                if let Some(eq) = children.iter().position(|&c| self.kind(c) == "=") {
                    let name = children[..eq]
                        .iter()
                        .copied()
                        .find_map(|c| {
                            if self.kind(c) == "identifier_list" {
                                self.children(c).into_iter().find(|&ic| self.kind(ic) == "identifier").map(|ic| self.text_string(ic).to_ascii_lowercase())
                            } else if self.kind(c) == "identifier" {
                                Some(self.text_string(c).to_ascii_lowercase())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();
                    if !name.is_empty() {
                        let mut parts = Vec::new();
                        for &c in &children[eq + 1..] {
                            let ck = self.kind(c);
                            if matches!(ck, "nil" | "true" | "false") {
                                parts.push(format!("[\"n\",\"{ck}\"]"));
                            } else if !self.is_layout(c) && ck != ";" {
                                let f = self.form(c);
                                if !f.is_empty() && f != "0" {
                                    parts.push(f);
                                }
                            }
                        }
                        let val = if parts.is_empty() {
                            "0".to_string()
                        } else if parts.len() == 1 {
                            parts.into_iter().next().unwrap()
                        } else {
                            format!("[\"m\",[{}]]", parts.join(","))
                        };
                        self.lexer_event(b'a', line, &[name], &val, false);
                        return;
                    }
                }
                let mut names: Vec<String> = Vec::new();
                let mut tparts: Vec<String> = Vec::new();
                let mut seen_colon = false;
                for c in children {
                    let ck = self.kind(c);
                    if ck == ":" {
                        seen_colon = true;
                        continue;
                    }
                    if self.is_layout(c) || matches!(ck, ";" | ",") {
                        continue;
                    }
                    if !seen_colon {
                        if ck == "identifier_list" {
                            for ic in self.children(c) {
                                if self.kind(ic) == "identifier" {
                                    names.push(self.text_string(ic).to_ascii_lowercase());
                                }
                            }
                        } else if ck == "identifier" {
                            names.push(self.text_string(c).to_ascii_lowercase());
                        }
                    } else {
                        let f = self.form(c);
                        if !f.is_empty() && f != "0" {
                            tparts.push(f);
                        }
                    }
                }
                if !names.is_empty() && !tparts.is_empty() {
                    let last = names.pop().unwrap();
                    let mut last_m = vec![{
                        let mut n = String::from(r#"["n","#);
                        push_string(&mut n, &last);
                        n.push(']');
                        n
                    }];
                    last_m.extend(tparts);
                    let tail = format!(r#"["m",[{}]]"#, last_m.join(","));
                    let val = if names.is_empty() {
                        tail
                    } else {
                        let mut tup: Vec<String> = names.into_iter().map(|nm| {
                            let mut n = String::from(r#"["n","#);
                            push_string(&mut n, &nm);
                            n.push(']');
                            n
                        }).collect();
                        tup.push(tail);
                        format!(r#"["t",[{}]]"#, tup.join(","))
                    };
                    self.lexer_event(b'x', line, &[], &val, false);
                }
            }
            "plain_case_item" | "parenthesized_pattern" | "test_expression" | "for_clause" if self.facts.tag == "SH" => {
                let line = self.line(id);
                let mut te_call_id: Option<usize> = None;
                if kind == "test_expression" {
                    let twords: Vec<(u32, String)> = self
                        .children(id)
                        .into_iter()
                        .filter(|&c| !matches!(self.kind(c), "whitespace" | "comment" | "[[" | "]]"))
                        .map(|c| (self.line(c), self.text_string(c)))
                        .filter(|(_, s)| !s.is_empty())
                        .collect();
                    for idx in 0..twords.len() {
                        let (wline, ref w) = twords[idx];
                        if w.starts_with('-') && w.len() > 1 && w.as_bytes()[1].is_ascii_alphabetic() {
                            let fname = w.trim_start_matches('-');
                            if idx + 1 < twords.len() && twords[idx + 1].0 == wline {
                                let cid = self.push_call(fname.to_string(), wline);
                                if te_call_id.is_none() {
                                    te_call_id = Some(cid);
                                }
                            }
                        }
                    }
                }
                let raw = if kind == "for_clause" || kind == "plain_case_item" {
                    let stop = if kind == "for_clause" { "do" } else { ")" };
                    let mut h = String::new();
                    for ch in self.children(id) {
                        if self.kind(ch) == stop {
                            break;
                        }
                        h.push(' ');
                        h.push_str(&self.text_string(ch));
                    }
                    h
                } else {
                    self.text_string(id)
                };
                let segs: Vec<&str> = raw
                    .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
                    .map(|s| s.trim_start_matches('-'))
                    .filter(|s| {
                        !s.is_empty()
                            && !matches!(*s, "for" | "select" | "in" | "do" | "done")
                            && s.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                    })
                    .collect();
                if !segs.is_empty() {
                    let inner = if segs.len() == 1 {
                        let mut n = String::from("[\"n\",");
                        push_string(&mut n, segs[0]);
                        n.push(']');
                        n
                    } else {
                        let parts: Vec<String> = segs
                            .iter()
                            .map(|s| {
                                let mut n = String::from("[\"n\",");
                                push_string(&mut n, s);
                                n.push(']');
                                n
                            })
                            .collect();
                        format!("[\"m\",[{}]]", parts.join(","))
                    };
                    let val = if kind == "test_expression" {
                        if let Some(cid) = te_call_id {
                            format!("[\"t\",[[\"t\",[[\"c\",{cid},0,[{inner}]]]]]]")
                        } else {
                            format!("[\"t\",[[\"t\",[{inner}]]]]")
                        }
                    } else if kind == "for_clause" {
                        let dq = raw.bytes().filter(|&b| b == b'"').count() / 2;
                        let sq = raw.bytes().filter(|&b| b == b'\'').count() / 2;
                        let has_sub = raw.contains("$(") || raw.contains('`');
                        let mut v = if dq + sq > 0 || has_sub {
                            format!("[\"m\",[{inner},0,0]]")
                        } else {
                            inner
                        };
                        if has_sub {
                            let nc = (raw.matches("$(").count() + raw.bytes().filter(|&b| b == b'|').count() + dq + sq + 1).min(6);
                            for _ in 0..nc {
                                v = format!("[\"c\",0,0,[{v}]]");
                            }
                        }
                        v
                    } else {
                        inner
                    };
                    self.lexer_event(b'x', line, &[], &val, false);
                }
            }
            "heredoc_body" if self.facts.tag == "SH" => {
                let start_line = self.line(id);
                let raw = self.text_string(id);
                for (offset, l) in raw.split('\n').enumerate() {
                    let t = l.trim();
                    if t.is_empty() || t.starts_with('#') {
                        continue;
                    }
                    let line = start_line + offset as u32;
                    if let Some((lhs_raw, rhs_raw)) = t.split_once('=') {
                        let lhs = lhs_raw.trim();
                        if !lhs.is_empty()
                            && lhs.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                            && lhs.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                        {
                            let segs: Vec<&str> = rhs_raw
                                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
                                .map(|s| s.trim_start_matches('-'))
                                .filter(|s| !s.is_empty() && s.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_'))
                                .collect();
                            let val = if segs.is_empty() {
                                "0".to_string()
                            } else if segs.len() == 1 {
                                let mut n = String::from("[\"n\",");
                                push_string(&mut n, segs[0]);
                                n.push(']');
                                n
                            } else {
                                let parts: Vec<String> = segs
                                    .iter()
                                    .map(|s| {
                                        let mut n = String::from("[\"n\",");
                                        push_string(&mut n, s);
                                        n.push(']');
                                        n
                                    })
                                    .collect();
                                format!("[\"m\",[{}]]", parts.join(","))
                            };
                            self.lexer_event(b'a', line, &[lhs.to_string()], &val, false);
                            continue;
                        }
                    }
                    let segs: Vec<&str> = t
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
                        .map(|s| s.trim_start_matches('-'))
                        .filter(|s| !s.is_empty() && s.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_'))
                        .collect();
                    if !segs.is_empty() {
                        let val = if segs.len() == 1 {
                            let mut n = String::from("[\"n\",");
                            push_string(&mut n, segs[0]);
                            n.push(']');
                            n
                        } else {
                            let parts: Vec<String> = segs
                                .iter()
                                .map(|s| {
                                    let mut n = String::from("[\"n\",");
                                    push_string(&mut n, s);
                                    n.push(']');
                                    n
                                })
                                .collect();
                            format!("[\"m\",[{}]]", parts.join(","))
                        };
                        self.lexer_event(b'x', line, &[], &val, false);
                    }
                }
            }
            "fi" | "done" | "esac" | "then" | "else" | "do" if self.facts.tag == "SH" => {
                self.lexer_event(b'x', self.line(id), &[], &format!("[\"n\",\"{kind}\"]"), false);
            }
            "simple_command" if self.facts.tag == "SH" => {
                let children = self.children(id);
                let line = self.line(id);
                // Helper closure to extract Bash identifier segments (preserving internal '-')
                let sh_segs = |text: &str| -> String {
                    let segs: Vec<&str> = text
                        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
                        .map(|s| s.trim_start_matches('-'))
                        .filter(|s| !s.is_empty() && s.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_'))
                        .collect();
                    if segs.is_empty() {
                        "0".to_string()
                    } else if segs.len() == 1 {
                        let mut n = String::from("[\"n\",");
                        push_string(&mut n, segs[0]);
                        n.push(']');
                        n
                    } else {
                        let parts: Vec<String> = segs
                            .iter()
                            .map(|s| {
                                let mut n = String::from("[\"n\",");
                                push_string(&mut n, s);
                                n.push(']');
                                n
                            })
                            .collect();
                        format!("[\"m\",[{}]]", parts.join(","))
                    }
                };

                // Extract identifiers/digits that the flat lexer sees outside quotes
                // when a double-quoted string contains `$(... "..." ...)`.
                let extract_nested_dq_calls = |text: &str| -> Vec<String> {
                    let mut out = Vec::new();
                    if !text.contains("$(") {
                        return out;
                    }
                    let b = text.as_bytes();
                    let mut quotes = Vec::new();
                    let mut i = 0;
                    while i < b.len() {
                        if b[i] == b'\\' && i + 1 < b.len() {
                            i += 2;
                            continue;
                        }
                        if b[i] == b'"' {
                            quotes.push(i);
                        }
                        i += 1;
                    }
                    if quotes.len() < 4 {
                        return out;
                    }
                    // Spans (quotes[1]+1 .. quotes[2]), (quotes[3]+1 .. quotes[4]), ...
                    let mut qi = 1;
                    while qi + 1 < quotes.len() {
                        let span = &text[quotes[qi] + 1..quotes[qi + 1]];
                        for tok in span.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-' && c != '.') {
                            let tok = tok.trim_matches(|c: char| c == '-' || c == '.');
                            if tok.is_empty() {
                                continue;
                            }
                            if tok.bytes().all(|ch| ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'-' || ch == b'.') {
                                out.push(tok.to_string());
                            }
                        }
                        qi += 2;
                    }
                    out
                };

                let wrap_sh_expr = |text: &str, mut base: String, cid: usize| -> String {
                    let has_cmdsub = text.contains("$(") || text.contains('`');
                    let dq = text.bytes().filter(|&b| b == b'"').count() / 2;
                    let sq = text.bytes().filter(|&b| b == b'\'').count() / 2;
                    let pipes = text.bytes().filter(|&b| b == b'|').count();
                    let subs = text.matches("$(").count() + text.bytes().filter(|&b| b == b'`').count() / 2;
                    let n_zeros = dq + sq + usize::from(has_cmdsub);
                    let n_calls = if has_cmdsub || cid != 0 {
                        (subs + pipes + dq + sq + 1).min(8)
                    } else {
                        0
                    };
                    if n_zeros > 0 {
                        let mut parts = vec![base];
                        for _ in 0..n_zeros.min(6) {
                            parts.push("0".to_string());
                        }
                        base = format!("[\"m\",[{}]]", parts.join(","));
                    }
                    for k in 0..n_calls {
                        let c = if k == 0 { cid } else { 0 };
                        base = format!("[\"c\",{c},0,[{base}]]");
                    }
                    base
                };

                // Check if this simple_command is a pure assignment (`VAR=val`, `VAR=(...)`, `emoji[k]=v`)
                let mut items: Vec<(u32, Option<String>, String, bool)> = Vec::new();
                let mut raw_words: Vec<String> = Vec::new();
                let mut pending_assign: Option<(u32, String, Vec<String>)> = None;

                for c in children {
                    let ck = self.kind(c);
                    let cline = self.line(c);
                    if matches!(ck, "whitespace" | "comment" | "heredoc_redirect") {
                        continue;
                    }
                    let t = self.text_string(c);
                    if t.is_empty() {
                        continue;
                    }
                    let only_decl_prefix = raw_words.iter().all(|w| {
                        matches!(w.as_str(), "local" | "export" | "declare" | "readonly" | "typeset")
                            || w.starts_with('-')
                    });
                    if pending_assign.is_none()
                        && only_decl_prefix
                        && (ck == "assignment_word"
                            || (ck == "word"
                                && t.contains('=')
                                && t.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')))
                    {
                        if let Some((lhs_raw, rhs_raw)) = t.split_once('=') {
                            let lhs_base = lhs_raw.split('[').next().unwrap_or(lhs_raw);
                            let lhs = lhs_base.trim_end_matches('+').trim().to_string();
                            if !lhs.is_empty() {
                                let mut rhs_parts = Vec::new();
                                let rhs_t = rhs_raw.trim();
                                if !rhs_t.is_empty() && !rhs_t.starts_with("$'") {
                                    let f = sh_segs(rhs_t);
                                    let dq_calls = extract_nested_dq_calls(rhs_t);
                                    let mut last_cid = 0usize;
                                    for cn in dq_calls {
                                        last_cid = self.push_call(cn, cline);
                                    }
                                    if last_cid != 0 || rhs_t.contains("$(") || rhs_t.contains('`') {
                                        rhs_parts.push(wrap_sh_expr(rhs_t, f, last_cid));
                                    } else if f != "0" {
                                        rhs_parts.push(wrap_sh_expr(rhs_t, f, 0));
                                    } else if let Some(d) = rhs_t.strip_prefix('$') {
                                        let d = d.trim_matches(|c| c == '{' || c == '}');
                                        if !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()) {
                                            rhs_parts.push(format!("[\"n\",\"{d}\"]"));
                                        }
                                    }
                                }
                                pending_assign = Some((cline, lhs, rhs_parts));
                                continue;
                            }
                        }
                    }
                    if let Some((aline, ref _lhs, ref mut rhs_parts)) = pending_assign {
                        if cline == aline && !t.starts_with("$'") {
                            let f = sh_segs(&t);
                            let dq_calls = extract_nested_dq_calls(&t);
                            let mut last_cid = 0usize;
                            for cn in dq_calls {
                                last_cid = self.push_call(cn, cline);
                            }
                            if last_cid != 0 || matches!(ck, "command_substitution" | "backtick_substitution") || t.contains("$(") || t.contains('`') {
                                rhs_parts.push(wrap_sh_expr(&t, f, last_cid));
                            } else if f != "0" {
                                rhs_parts.push(wrap_sh_expr(&t, f, 0));
                            } else if let Some(d) = t.strip_prefix('$') {
                                let d = d.trim_matches(|c| c == '{' || c == '}');
                                if !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()) {
                                    rhs_parts.push(format!("[\"n\",\"{d}\"]"));
                                }
                            }
                            continue;
                        }
                    }
                    raw_words.push(t.clone());
                    if matches!(ck, "variable" | "parameter_expansion") {
                        if let Some(vn) = Self::tcl_var_name(&t) {
                            let mut n = String::from("[\"n\",");
                            push_string(&mut n, &vn);
                            n.push(']');
                            items.push((cline, Some(vn), n, true));
                        } else {
                            items.push((cline, None, "0".to_string(), true));
                        }
                    } else if ck == "string_literal" {
                        let inner = t.trim_matches(|ch| ch == '"' || ch == '\'');
                        let ib = inner.as_bytes();
                        let mut qvars: Vec<String> = Vec::new();
                        if t.starts_with('"') {
                            let mut qj = 0;
                            while qj < ib.len() {
                                if ib[qj] == b'\\' && qj + 1 < ib.len() {
                                    qj += 2;
                                    continue;
                                }
                                if ib[qj] == b'$' {
                                    if let Some(vn) = Self::tcl_var_name(&inner[qj..]) {
                                        let clean_vn = vn.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').next().unwrap_or(&vn);
                                        if !clean_vn.is_empty() {
                                            let mut n = String::from("[\"n\",");
                                            push_string(&mut n, clean_vn);
                                            n.push(']');
                                            qvars.push(n);
                                        }
                                    }
                                }
                                qj += 1;
                            }
                        }
                        let dq_calls = extract_nested_dq_calls(&t);
                        let qform = if !dq_calls.is_empty() {
                            let mut last_cid = 0usize;
                            for cn in dq_calls {
                                last_cid = self.push_call(cn, cline);
                            }
                            let inner_f = if qvars.is_empty() {
                                sh_segs(&t)
                            } else if qvars.len() == 1 {
                                qvars.remove(0)
                            } else {
                                format!("[\"m\",[{}]]", qvars.join(","))
                            };
                            wrap_sh_expr(&t, inner_f, last_cid)
                        } else if qvars.is_empty() {
                            "0".to_string()
                        } else if qvars.len() == 1 {
                            qvars.remove(0)
                        } else {
                            format!("[\"m\",[{}]]", qvars.join(","))
                        };
                        items.push((cline, None, qform, true));
                    } else if matches!(ck, "command_substitution" | "backtick_substitution" | "process_substitution" | "arithmetic") {
                        let f = sh_segs(&t);
                        let cf = if matches!(ck, "command_substitution" | "backtick_substitution") {
                            wrap_sh_expr(&t, f, 0)
                        } else {
                            f
                        };
                        items.push((cline, None, cf, false));
                    } else {
                        let is_flag = t.starts_with('-') && t.len() > 1 && t.as_bytes()[1].is_ascii_alphabetic();
                        let norm = t.trim_start_matches('-').to_string();
                        let clean_tail = if norm.starts_with("zsh/") {
                            norm.rsplit('/').next().unwrap_or(&norm)
                        } else {
                            norm.as_str()
                        };
                        let is_ident = ck != "redirect"
                            && clean_tail.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                            && clean_tail.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.');
                        let seg_form = sh_segs(&t);
                        const SH_NOISE: &[&str] = &[
                            "await", "yield", "new", "not", "typeof", "delete", "void", "throw", "raise", "async",
                            "lambda", "in", "is", "and", "or", "if", "else", "for", "of", "as", "then", "do",
                            "instanceof", "sizeof", "return", "my", "local", "let", "var", "const", "final",
                            "mut", "ref", "unsafe", "defer", "go", "our", "pub",
                        ];
                        if !is_flag && SH_NOISE.contains(&clean_tail) {
                            items.push((cline, None, "0".to_string(), false));
                        } else if is_ident {
                            let cname = if norm.contains('/') { clean_tail.to_string() } else { norm.clone() };
                            let is_val = !is_flag && t.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
                            items.push((cline, Some(cname), seg_form, is_val));
                        } else {
                            let is_val = !is_flag && ck != "redirect" && t.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
                            items.push((cline, None, seg_form, is_val));
                        }
                    }
                }
                if let Some((aline, lhs, mut rhs_parts)) = pending_assign {
                    let rhs_form = if rhs_parts.is_empty() {
                        "0".to_string()
                    } else if rhs_parts.len() == 1 {
                        rhs_parts.remove(0)
                    } else {
                        format!("[\"m\",[{}]]", rhs_parts.join(","))
                    };
                    self.lexer_event(b'a', aline, &[lhs], &rhs_form, false);
                }
                if let Some(first) = raw_words.first() {
                    if matches!(first.as_str(), "source" | ".") {
                        if let Some(target) = raw_words.get(1) {
                            let module = target.trim_matches(|c| c == '"' || c == '\'').to_string();
                            let last = module.rsplit('/').next().unwrap_or(&module).to_string();
                            if !module.is_empty() {
                                self.facts.imports.push(Import { line, module: module.clone(), alias: last.clone(), kind: IMPORT_STAR, ..Import::default() });
                                if let Some((_, ext)) = last.rsplit_once('.') {
                                    if !ext.is_empty() {
                                        self.facts.imports.push(Import { line, module, alias: ext.to_string(), kind: IMPORT_STAR, ..Import::default() });
                                    }
                                }
                            }
                        }
                    }
                }
                if !items.is_empty() {
                    let mut acc = String::new();
                    for idx in (0..items.len()).rev() {
                        let (wline, ref ident_opt, ref form_str, _) = items[idx];
                        let next_is_val = idx + 1 < items.len() && items[idx + 1].3 && items[idx + 1].0 == wline;
                        if acc.is_empty() {
                            acc = form_str.clone();
                        } else if let Some(cname) = ident_opt {
                            if next_is_val {
                                let cid = self.push_call(cname.clone(), wline);
                                if form_str.starts_with("[\"m\",") {
                                    acc = format!("[\"m\",[{form_str},[\"c\",{cid},0,[{acc}]]]]");
                                } else {
                                    acc = format!("[\"c\",{cid},0,[{acc}]]");
                                }
                            } else if idx == 0 {
                                let _ = self.push_call(cname.clone(), wline);
                                acc = format!("[\"m\",[{form_str},{acc}]]");
                            } else {
                                acc = format!("[\"m\",[{form_str},{acc}]]");
                            }
                        } else {
                            acc = format!("[\"m\",[{form_str},{acc}]]");
                        }
                    }
                    if items.len() == 1 {
                        if let Some(ref cname) = items[0].1 {
                            self.push_call(cname.clone(), items[0].0);
                        }
                    }
                    if raw_words.first().is_some_and(|w| w == "return") {
                        self.lexer_event(b'r', line, &[], "0", false);
                    } else if !acc.is_empty() && acc != "0" {
                        self.lexer_event(b'x', line, &[], &acc, false);
                    if let Some(src_line) = std::str::from_utf8(self.src).ok().and_then(|s| s.lines().nth(line.saturating_sub(1) as usize)) {
                        let sl = src_line.split(" #").next().unwrap_or(src_line).trim();
                        if !sl.is_empty() && !sl.starts_with('#') && (sl.contains('|') || sl.contains("&&") || sl.contains("||") || sl.contains('.') || sl.contains("${") || sl.contains(';')) {
                            let mut lparts: Vec<String> = Vec::new();
                            for tok in sl.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-' && c != '.') {
                                let tok = tok.trim_start_matches('-').trim_matches('.');
                                if tok.is_empty() {
                                    continue;
                                }
                                if tok.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                                    let mut n = String::from("[\"n\",");
                                    push_string(&mut n, tok);
                                    n.push(']');
                                    lparts.push(n);
                                    if tok.contains('.') {
                                        for seg in tok.split('.') {
                                            let seg = seg.trim_start_matches('-');
                                            if !seg.is_empty() && seg.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                                                let mut n2 = String::from("[\"n\",");
                                                push_string(&mut n2, seg);
                                                n2.push(']');
