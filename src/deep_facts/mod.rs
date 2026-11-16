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
const CS_CALLS: &[&str] = &["invocation_expression", "object_creation_expression", "object_creation"];
const CS_IMPORTS: &[&str] = &["using_directive"];

const KT_DEFS: &[Def] = &[
    Def { kind: "class_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "interface_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_class_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "object_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "function_declaration", name: &["function_head"], params: Some("parameter_list"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const KT_CALLS: &[&str] = &["call_expression"];
const KT_IMPORTS: &[&str] = &["import_header", "package_header"];

const SC_DEFS: &[Def] = &[
    Def { kind: "class_definition", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "trait_definition", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "object_definition", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "def_definition", name: &["identifier"], params: Some("parameter_clause"), params_nth: 0, name_last: false, receiver: false, container: false },
];
const SC_CALLS: &[&str] = &["call", "call_expression"];
const SC_IMPORTS: &[&str] = &["import_declaration"];

const SW_DEFS: &[Def] = &[
    Def { kind: "class_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "struct_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "protocol_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "extension_declaration", name: &["identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "function_declaration", name: &["identifier"], params: Some("parameter_clause"), params_nth: 0, name_last: false, receiver: false, container: false },
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
        Lang::Java => Kinds { definitions: JAVA_DEFS, calls: &[], imports: JAVA_IMPORTS, soft: &[], family: Family::Java },
        Lang::CSharp => Kinds { definitions: CS_DEFS, calls: CS_CALLS, imports: CS_IMPORTS, soft: &[], family: Family::Plain },
        Lang::Kotlin => Kinds { definitions: KT_DEFS, calls: KT_CALLS, imports: KT_IMPORTS, soft: &[], family: Family::Plain },
        Lang::Scala => Kinds { definitions: SC_DEFS, calls: SC_CALLS, imports: SC_IMPORTS, soft: &[], family: Family::Plain },
        Lang::Swift => Kinds { definitions: SW_DEFS, calls: SW_CALLS, imports: SW_IMPORTS, soft: &[], family: Family::Plain },
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
    /// The form text a call node already produced, by node id. A block value is
    /// read twice — once for the value the assignment carries, once when the
    /// walk reaches the statements inside it — and the second read pushed a
    /// second `Call` for the same source call. Returning the first text keeps
    /// one call and one index, which is what the text in the event refers to
    /// (P5-11).
    call_texts: std::collections::HashMap<u32, String>,
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
        // JSON stays out: a data file has no symbols, calls or imports to
        // read, so the CST would add the module symbol the lexer already
        // writes and nothing else. See `tests/facts_rust_json.rs`.
        _ => None,
    }
}

/// Read a file's facts off its CST.
pub fn extract(root: &[u8], tree: &Cst, lang: Lang) -> Facts {
    let mut walker = Walker::new(root, tree, lang);
    walker.run();
    walker.facts
}

impl<'a> Walker<'a> {
    pub(crate) fn new(src: &'a [u8], tree: &'a Cst, lang: Lang) -> Self {
        let mut facts = Facts { tag: tag(lang).to_string(), ..Facts::default() };
        facts.symbols.push(Symbol { key: "<module>".into(), kind: KIND_MODULE, line: 1, ..Symbol::default() });
        Walker {
            src,
            nodes: &tree.nodes,
            language: lang.language(),
            kinds: kinds(lang),
            line_starts: line_starts(src),
            facts,
            stack: vec![Frame { node: tree.root, symbol: 0, class: String::new(), receiver: String::new(), container: false, defines: None }],
            bound: Vec::new(),
            consumed: Vec::new(),
            call_texts: std::collections::HashMap::new(),
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
    pub(crate) fn leaf_name(&self, id: u32) -> Option<String> {
        let node = self.node(id);
        if node.first_child != NONE {
            return None;
        }
        let text = self.text(id);
        let first = *text.first()?;
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return None;
        }
        if !text.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$') {
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
                // A regex literal is a value, not punctuation: the lexer
                // writes it as the constant 0, and dropping it as layout lost
                // one argument from every call that takes a regex
                // (`arg.replace(/x/, 's')`, 69 js first-loss rows).
                | "regex"
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
        !(text[0].is_ascii_alphanumeric() || text[0] == b'_' || text[0] == b'"' || text[0] == b'\'')
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
        if !matches!(self.kind(id), "call" | "call_expression" | "new_expression") {
            return false;
        }
        match self.children(id).into_iter().find(|&child| {
            !matches!(self.kind(child), "new" | "argument_list" | "arguments" | "type_arguments" | "(" | ")")
        }) {
            Some(callee) => {
                callee != id && !matches!(self.kind(callee), "call" | "call_expression" | "new_expression")
            }
            None => false,
        }
    }

    pub(crate) fn push_call(&mut self, path: String, line: u32) -> usize {
        let symbol = self.frame().symbol;
        let calls = &mut self.facts.symbols[symbol].calls;
        calls.push(Call { path, line });
        calls.len() - 1
    }

    // -- enter / leave ------------------------------------------------------

    pub(crate) fn enter(&mut self, id: u32) {
        let kind = self.kind(id);
        if self.is_rust() && self.enter_rust(id) {
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
        // one. Read before the definition table claims the node.
        if kind == "method_declaration"
            && self.kinds.calls.contains(&"object_creation_expression")
            && !self.has_kind(id, &["block"])
        {
            let children = self.children(id);
            let name = children
                .iter()
                .copied()
                .take_while(|&child| !matches!(self.kind(child), "parameter_list" | "formal_parameters"))
                .filter(|&child| self.kind(child) == "identifier")
                .last()
                .map(|node| self.text_string(node));
            if let Some(name) = name {
                let line = self.line(id);
                let index = self.push_call(name, line);
                // The line is an `x` event whose last element is that call,
                // with every parameter read as one merge of its words:
                // `m:[n:public,n:abstract,n:T,c(0;[m:[n:Type,n:objectType]])]`.
                let mut parts: Vec<String> = Vec::new();
                for child in &children {
                    if matches!(self.kind(*child), "parameter_list" | "formal_parameters") {
                        break;
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
                        let words = self.c_words_of(parameter);
                        if words.is_empty() {
                            continue;
                        }
                        let inner: Vec<String> = words
                            .iter()
                            .map(|word| {
                                let mut form = String::from("[\"n\",");
                                push_string(&mut form, word);
                                form.push(']');
                                form
                            })
                            .collect();
                        args.push(format!("[\"m\",[{}]]", inner.join(",")));
                    }
                }
                let call = format!("[\"c\",{index},0,[{}]]", args.join(","));
                if parts.is_empty() {
                    parts.push(call);
                } else {
                    parts.push(call);
                }
                let value = format!("[\"m\",[{}]]", parts.join(","));
                self.lexer_event(b'x', line, &[], &value, false);
            }
        }
        if let Some(def) = self.kinds.definitions.iter().find(|def| def.kind == kind) {
            self.open_definition(id, def);
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
            "field_declaration" | "event_field_declaration" => {
                // A field initialised with a call is an assignment to the
                // lexer; anything else is the header event.
                if !self.cs_field_assignment(id) {
                    self.decl_statement(id);
                    self.field_initializer_calls(id);
                }
                self.cs_declared_vars(id);
            }
            // `var o = JObject.Parse(text);`: a C# local initialised with a call
            // recorded nothing at all - deep 0 against lexer 2 on a two-line
            // method. Only C# spells its call kinds this way, so the arm is
            // gated on them rather than on the family.
            "local_variable_declaration" if self.kinds.calls.contains(&"object_creation_expression") => {
                if !self.cs_field_assignment(id) {
                    self.field_initializer_calls(id);
                }
                self.cs_declared_vars(id);
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
            "indexer_declaration" => self.java_header_event(id),
            "property_declaration" => {
                if self.kinds.calls.contains(&"object_creation_expression") {
                    self.java_header_event(id);
                } else {
                    self.kt_property(id);
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
            "annotation" if self.kinds.family == Family::Plain => {
                if let Some(node) = self.children(id).into_iter().find(|&child| matches!(self.kind(child), "identifier")) {
                    let name = self.text_string(node);
                    let line = self.line(id);
                    let index = self.push_call(name, line);
                    // The lexer files the annotation's own arguments as an
                    // event holding the call's form, as it does for any other
                    // call written as a statement.
                    if let Some(list) = self.children(id).into_iter().find(|&child| self.kind(child) == "annotation_arguments") {
                        if self.children(list).iter().any(|&child| !self.is_layout(child)) {
                            let args = self.arguments(list);
                            self.event(&format!("[\"x\",{line},[\"c\",{index},0,{args}]]"));
                        }
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
                    let mut value = "[\"n\",".to_string();
                    push_string(&mut value, &name);
                    value.push(']');
                    self.lexer_event(b'x', line, &[], &value, false);
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
                                match (self.first_identifier(*member), self.enum_member_value(*member)) {
                                    (Some(member_name), value) if value == "0" => {
                                        let mut form = String::from("[\"n\",");
                                        push_string(&mut form, &member_name);
                                        form.push(']');
                                        parts.push(form);
                                    }
                                    (Some(member_name), value) => {
                                        let mut key = String::from("[\"n\",");
                                        push_string(&mut key, &member_name);
                                        key.push(']');
                                        parts.push(format!("[\"m\",[{key},{value}]]"));
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
            "expression_statement" => self.expression_statement(id),
            // Scala's `def f(): T = expr` writes the body as a `def_body` whose
            // child is the expression itself. Nothing read that node, so a call
            // in an expression-bodied def never reached the record.
            "def_body" => self.expression_statement(id),
            "return_statement" => self.return_statement(id),
            "variable_declarator" if self.is_js() => self.declarator(id),
            "assignment_expression" if self.is_js() => self.assignment_definition(id),
            "public_field_definition" | "field_definition" | "property_signature" if self.is_js() => {
                self.field(id)
            }
            "throw_statement" => self.expression_statement(id),
            // `@dec` / `@Component({...})` decorates the declaration that
            // follows; in JS/TS it is not a statement. Read as an expression
            // statement it wrote an assignment of `dec` on the decorator line,
            // and the lexer records only the name there, so the decorated
            // member's event had no counterpart (the last TS construct in
            // P4-02). Python spells a decorator with the same node kind but its
            // reader does model the line, so only the JS family skips it.
            "decorator" => {
                if !self.is_js() {
                    self.expression_statement(id);
                }
            }
            // `raise E(x) from c(y)`, `assert a(v), b(v)`, `del xs[k(v)]`:
            // every value-bearing child is read, not only the first.
            // `raise ConfigError(f"..")`: the form fills the call table, but
            // the statement itself emitted nothing, so the one-pass reader's
            // event for it had no counterpart.
            "raise_statement" => {
                let mut parts = Vec::new();
                for child in self.children(id) {
                    if self.is_layout(child) || self.kind(child) == "raise" {
                        continue;
                    }
                    let form = self.form(child);
                    if form != "0" {
                        parts.push(form);
                    }
                }
                if !parts.is_empty() {
                    let line = self.line(id);
                    self.event(&format!("[\"x\",{line},[\"m\",[{}]]]", parts.join(",")));
                }
            }
            "assert_statement" | "delete_statement" => self.statement_values(id),
            // `pass`, `break`, `continue`: the one-pass lexer reads the word as
            // an expression and files an `x` event for it, so the CST does too.
            // The module symbol is emitted only when it has events, so losing
            // the event for a module-level `pass` dropped the whole symbol.
            "pass_statement" | "break_statement" | "continue_statement" => {
                let line = self.line(id);
                // The node's text carries the statement terminator: a JS
                // `break;` was written as the name `break;` while the lexer
                // wrote `break`, and a name the lexer read but the CST did not
                // is a fallback with nothing wrong underneath it. The word is
                // the first run of text before the `;` or the newline.
                let raw = self.text_string(id);
                let word = raw
                    .split(|ch: char| ch == ';' || ch == '\n' || ch.is_whitespace())
                    .next()
                    .unwrap_or("")
                    .to_string();
                let label = self
                    .children(id)
                    .into_iter()
                    .skip(1)
                    .find_map(|child| self.leaf_name(child));
                if let Some(label) = label {
                    let mut text = format!("[\"x\",{line},[\"m\",[[\"n\",");
                    push_string(&mut text, &word);
                    text.push_str("],[\"n\",");
                    push_string(&mut text, &label);
                    text.push_str("]]]]");
                    self.event(&text);
                } else {
                    let mut text = format!("[\"x\",{line},[\"n\",");
                    push_string(&mut text, &word);
                    text.push_str("]]");
                    self.event(&text);
                }
            }
            "if_statement" => {
                self.condition_before(id);
                self.event("[\"b\"]");
            }
            "elif_clause" | "else_clause" => {
                self.event("[\"e\"]");
                if kind == "elif_clause" {
                    self.condition_before(id);
                } else {
                    // `["f"]`: an arm with no condition, so the branches cover
                    // every case. The lexer writes it for a bare `else`; the
                    // events are the same fact either way.
                    self.event("[\"f\"]");
                }
            }
            "with_statement" => {
                // `with A(), B() as f:` has one value per context manager. The
                // bound one carries the assignment event; every other is still
                // an expression whose calls are facts, and reading only the
                // first dropped `open()` from `with nullcontext(), open(p)`.
                self.event("[\"b\"]");
                let bound = self.loop_binding(id);
                let bound_value = bound.as_ref().map(|(_, value)| *value);
                if let Some((targets, value)) = bound {
                    self.assignment_event(targets, value, self.line(id), false);
                    for child in self.children(id) {
                        if Some(child) == bound_value
                            || self.is_layout(child)
                            || matches!(self.kind(child), "block" | "statement_block")
                        {
                            continue;
                        }
                        let value = self.form(child);
                        if value != "0" {
                            let line = self.line(child);
                            self.event(&format!("[\"x\",{line},{value}]"));
                        }
                    }
                } else {
                    // `with self._lock:` binds nothing, and `with a, b:` is read
                    // by the lexer as one tuple event on the header line.
                    let mut values = Vec::new();
                    for child in self.children(id) {
                        if self.is_layout(child) || matches!(self.kind(child), "block" | "statement_block") {
                            continue;
                        }
                        let value = self.form(child);
                        if value != "0" {
                            values.push(value);
                        }
                    }
                    let line = self.line(id);
                    match values.len() {
                        0 => {}
                        1 => self.event(&format!("[\"x\",{line},{}]", values[0])),
                        _ => self.event(&format!("[\"x\",{line},[\"t\",[{}]]]", values.join(","))),
                    }
                }
            }
            "for_statement" | "for_in_statement" => {
                self.event("[\"b\"]");
                if let Some((target, value)) = self.loop_binding(id) {
                    self.assignment_event(target, value, self.line(id), false);
                } else if let Some(header) = self.child_of_kind(id, "for_header") {
                    let inner = self.children(header);
                    if let Some(semi) = inner.iter().position(|&c| self.kind(c) == ";") {
                        // `for (i = length; …)`: an init that is not a
                        // declaration is an assignment, and the lexer files it
                        // as one. Only looking after the first `;` dropped the
                        // target and its value. The grammar wraps the init in
                        // an `expressions` node, so the assignment is one level
                        // down.
                        for &child in &inner[..semi] {
                            self.for_init(child);
                        }
                        for &child in &inner[semi + 1..] {
                            if !self.is_layout(child) {
                                let value = self.form(child);
                                if value != "0" {
                                    let line = self.line(child);
                                    self.event(&format!("[\"x\",{line},{value}]"));
                                }
                            }
                        }
                    }
                }
            }
            "while_statement" => {
                self.condition_before(id);
                self.event("[\"b\"]");
            }
            "try_statement" | "match_statement" | "do_statement" => self.event("[\"b\"]"),
            // `switch (x)` carries a condition the one-pass reader writes as an
            // expression on the header line; the clause events below are the
            // body. Reading the body alone dropped the scrutinee, which is a
            // fact the resolver keeps (a call in the test is a call).
            "switch_statement" => {
                self.condition_before(id);
                self.event("[\"b\"]");
            }
            "catch_clause" | "finally_clause" | "except_clause" | "switch_case" | "case_clause" => {
                // `except ValueError:` names a type the one-pass reader files
                // as a value on the clause's line; the clause emits only its
                // terminator, so that name had no counterpart at all.
                // `except OSError as error:` binds a name, and the lexer
                // reads the clause as an assignment of the type to it.
                if kind == "catch_clause" {
                    // `catch (e)`: the lexer reads the caught binding as the
                    // clause's value. The binding sits inside a
                    // `catch_parameter`, so the name is one level down.
                    let mut names = Vec::new();
                    for child in self.children(id) {
                        names.extend(self.pattern_names(child));
                    }
                    if let Some(name) = names.into_iter().next() {
                        let line = self.line(id);
                        let mut text = format!("[\"x\",{line},[\"n\",");
                        push_string(&mut text, &name);
                        text.push_str("]]");
                        self.event(&text);
                    }
                }
                if kind == "except_clause" {
                    if let Some((targets, value)) = self.loop_binding(id) {
                        self.assignment_event(targets, value, self.line(id), false);
                        self.event("[\"e\"]");
                        return;
                    }
                    let mut parts = Vec::new();
                    for child in self.children(id) {
                        if self.is_layout(child) || matches!(self.kind(child), "except" | ":") {
                            continue;
                        }
                        if matches!(self.kind(child), "block" | "statement_block") {
                            break;
                        }
                        let form = self.form(child);
                        if form != "0" {
                            parts.push(form);
                        }
                    }
                    if !parts.is_empty() {
                        let line = self.line(id);
                        self.event(&format!("[\"x\",{line},[\"m\",[{}]]]", parts.join(",")));
                    }
                }
                self.event("[\"e\"]")
            }
            _ => {}
        }
    }

    pub(crate) fn leave(&mut self, id: u32) {
        let kind = self.kind(id);
        if matches!(self.kinds.family, Family::Go | Family::Java) {
            if self.frame().node == id {
                let frame = self.stack.pop().expect("a frame to pop");
                if let Some(index) = frame.defines {
                    self.facts.symbols[index].end_line = self.end_line(id);
                }
            }
            return;
        }
        // `(a) => f(a)`: the body is an expression, not a block, so nothing in
        // the walk below would read it. It is this function's return value.
        if kind == "arrow_function" && self.frame().node == id {
            if let Some(&body) = self.children(id).last() {
                if !matches!(self.kind(body), "statement_block" | "block") {
                    let line = self.line(id);
                    let value = self.form(body);
                    self.event(&format!("[\"r\",{line},{value}]"));
                }
            }
        }
        if self.is_rust() {
            self.leave_rust(id);
        }
        if matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "for_in_statement"
                | "while_statement"
                | "do_statement"
                | "try_statement"
                | "switch_statement"
                | "match_statement"
                | "with_statement"
        ) {
            self.event("[\"j\"]");
            if kind == "do_statement" {
                if let Some(cond) = self
                    .children(id)
                    .into_iter()
                    .rev()
                    .find(|&c| !self.is_layout(c) && !matches!(self.kind(c), "statement_block" | "block"))
                {
                    let value = self.form(cond);
                    if value != "0" {
                        let line = self.line(cond);
                        self.event(&format!("[\"x\",{line},{value}]"));
                    }
                }
            }
        }
        if self.frame().node != id {
            return;
        }
        let frame = self.stack.pop().expect("a frame to pop");
        if let Some(index) = frame.defines {
            self.facts.symbols[index].end_line = self.end_line(id);
        }
    }

    pub(crate) fn is_js(&self) -> bool {
        self.kinds.family == Family::Js
    }

    /// `true` for Kotlin: it is the only grammar here whose imports table
    /// carries the package clause, and the walk needs to know before it reads
    /// `property_declaration`, which Swift also uses.
    pub(crate) fn is_kotlin(&self) -> bool {
        self.kinds.imports.iter().any(|kind| *kind == "package_header")
    }

    /// `true` for Rust: `impl` names itself from its type, `use` is a path
    /// import, and the flow events are expressions, not statements.
    pub(crate) fn is_rust(&self) -> bool {
        self.kinds.family == Family::Rust
    }

    // -- definitions --------------------------------------------------------

    pub(crate) fn open_definition(&mut self, id: u32, def: &Def) {
        let line = self.line(id);
        if def.container {
            let name = self.def_name(id, def).unwrap_or_default();
            if name.is_empty() {
                return;
            }
            let bases = self.bases(id);
            let index = self.add_symbol(name.clone(), String::new(), KIND_CLASS, line, Vec::new());
            self.facts.symbols[index].bases = bases;
            // A class body writes onto the enclosing symbol; only its methods
            // take the class name as an owner.
            let symbol = self.frame().symbol;
            self.stack.push(Frame { node: id, symbol, class: name, receiver: String::new(), container: true, defines: Some(index) });
            // Java: when an annotation sits in front of it the lexer does not
            // see a class and reads the header as an expression statement.
            if self.kinds.family == Family::Java {
                self.java_header_event(id);
            }
            // C# and the brace languages beside it: `public enum ExclusionFilters`
            // is a statement to the lexer, and the tree wrote only the symbol, so
            // the header event had no counterpart on nearly every file.
            if matches!(self.language.name, "csharp" | "kotlin" | "scala" | "swift" | "objc") {
                self.java_header_event(id);
            }
            return;
        }
        // A computed property name on a method (`[expr]() {}`) is evaluated in
        // the enclosing scope when the class or object is defined, not inside
        // the method's own frame.
        if let Some(computed) = self.child_of_kind(id, "computed_property_name") {
            for child in self.children(computed) {
                if !self.is_layout(child) {
                    let value = self.form(child);
                    if value != "0" {
                        let at = self.line(child);
                        self.event(&format!("[\"x\",{at},{value}]"));
                    }
                }
            }
        }
        let named = self
            .def_name(id, def)
            .or_else(|| {
                let at = self.bound.iter().position(|(node, _)| *node == id)?;
                Some(self.bound.remove(at).1)
            });
        let name = named.unwrap_or_else(|| format!("<anonymous>@{line}"));
        let owner = if self.frame().container { self.frame().class.clone() } else { String::new() };
        // Go: a method's receiver decides its owner, and the receiver's name is
        // what the lexer spells `self` for inside the body.
        let (receiver, receiver_owner) = if def.receiver {
            def.params
                .map(|kind| self.nth_child_of_kind(id, kind, 0))
                .flatten()
                .map(|list| self.go_receiver(list))
                .unwrap_or_default()
        } else {
            (String::new(), String::new())
        };
        let owner = if receiver_owner.is_empty() { owner } else { receiver_owner };
        let mut params = def
            .params
            .map(|kind| self.nth_child_of_kind(id, kind, def.params_nth as usize))
            .flatten()
            .map(|node| self.params(id, node))
            .unwrap_or_default();
        // `x = lambda k: f(k)`: the lambda's own parameter list is where the
        // names are, since this binding carries no `parameters` rule.
        if params.is_empty() {
            if let Some(list) = self.child_of_kind(id, "lambda_parameters") {
                params = self.params(id, list);
            }
        }
        if params.first().is_some_and(|first| matches!(first.as_str(), "self" | "cls" | "this")) {
            params.remove(0);
        }
        let index = self.add_symbol(name, owner.clone(), KIND_FUNCTION, line, params);
        self.stack.push(Frame { node: id, symbol: index, class: owner, receiver, container: false, defines: Some(index) });
        // A binding whose body is a lambda (`x = lambda k: f(k)`) is a
        // definition the grammar writes as one node: nothing else in the walk
        // reads that lambda, so its body is read here, as this symbol's return.
        if let Some(lambda) = self.child_of_kind(id, "lambda") {
            if let Some(body) = self.children(lambda).into_iter().rev().find(|&child| !self.is_layout(child)) {
                let at = self.line(lambda);
                let value = self.form(body);
                if value != "0" {
                    self.event(&format!("[\"r\",{at},{value}]"));
                }
            }
        }
    }

    /// The name a definition node carries, if any.
    pub(crate) fn def_name(&self, id: u32, def: &Def) -> Option<String> {
        // Rust: a function named with a weak keyword (`fn union`,
        // `fn default`) has no identifier node — the grammar reads the word as
        // the keyword it is elsewhere — and the generic lookup then fell
        // through to the return type, naming the method `bool`. The name is
        // the leaf right after `fn`.
        if self.kinds.family == Family::Rust && matches!(def.kind, "function_item" | "function_signature_item") {
            let children = self.children(id);
            if let Some(at) = children.iter().position(|&child| self.text_string(child) == "fn") {
                if let Some(&name) = children.get(at + 1) {
                    let text = self.text_string(name);
                    if !text.is_empty() && !text.contains('(') && !text.contains(' ') {
                        return Some(text);
                    }
                }
            }
        }
        if def.name_last {
            // Java writes the return type before the name: `Item find(int id)`.
            let node = self
                .children(id)
                .into_iter()
                .filter(|&child| def.name.contains(&self.kind(child)))
                .next_back()?;
            return Some(self.text_string(node));
        }
        if let Some(node) = def.name.iter().find_map(|kind| self.child_of_kind(id, *kind)) {
            return Some(self.text_string(node));
        }
        // A function or method called `get` or `set` reads as the modifier that
        // introduces an accessor, so the grammar gives the name that anonymous
        // token rather than an identifier: `function set(a) {}` and
        // `class A { set(v) {} }` both came out unnamed while `setter` did not.
        // The name is then the leaf just before the parameter list — but only
        // that leaf: `const readBlob = async function* (blob) {}` puts a `*`
        // and the `function` keyword there, and naming the definition after
        // either one hides the assignment name that used to fill it.
        if !matches!(
            def.kind,
            "method_definition"
                | "function_declaration"
                | "generator_function_declaration"
                | "function_expression"
        ) {
            return None;
        }
        let params = def.params?;
        let children = self.children(id);
        let at = children.iter().position(|&child| self.kind(child) == params)?;
        let mut name = *children.get(at.checked_sub(1)?)?;
        // `get`, `set` and `async` ARE the name here; the words to step over are
        // the ones the grammar writes between a declaration and its parameters.
        if matches!(self.text_string(name).as_str(), "*" | "function" | "static") {
            let before = *children.get(at.checked_sub(2)?)?;
            if matches!(self.text_string(before).as_str(), "function" | "async") {
                return None;
            }
            name = before;
        }
        if self.node(name).first_child != NONE {
            return None;
        }
        let text = self.text_string(name);
        (!text.is_empty()).then_some(text)
    }

    pub(crate) fn bases(&self, id: u32) -> Vec<String> {
        let mut out = Vec::new();
        // Java writes the superclass and the interfaces in two clauses, so every
        // heritage child is read, not only the first.
        let heritage: Vec<u32> = self
            .children(id)
            .into_iter()
            .filter(|&child| {
                matches!(
                    self.kind(child),
                    "argument_list" | "class_heritage" | "implements_clause" | "superclass" | "super_interfaces"
                        | "extends_interfaces"
                )
            })
            .collect();
        if heritage.is_empty() {
            return out;
        }
        // Source order: the stack pops the last pushed first.
        let mut walk: Vec<u32> = heritage.into_iter().rev().collect();
        while let Some(node) = walk.pop() {
            for child in self.children(node) {
                match self.kind(child) {
                    "identifier" | "type_identifier" => out.push(self.text_string(child)),
                    "attribute" | "member_expression" | "nested_type_identifier" => {
                        if let Some(last) = self.last_name(child) {
                            out.push(last);
                        }
                    }
                    "(" | ")" | "," | ":" | "extends" | "implements" | "with" | "argument_list" | "class_heritage"
                    | "implements_clause" | "type_arguments" => walk.push(child),
                    _ => walk.push(child),
                }
            }
        }
        out
    }

    /// The leftmost identifier under a node: for a generic type that is the
    /// constructor, which is the name the type goes by — `JsonString<LogEntry>`
    /// is `JsonString`, not the argument inside the angle brackets.
    pub(crate) fn first_name(&self, id: u32) -> Option<String> {
        let mut walk = vec![id];
        while let Some(node) = walk.pop() {
            let kind = self.kind(node);
            if matches!(kind, "identifier" | "property_identifier" | "type_identifier") {
                return Some(self.text_string(node));
            }
            let mut kids = Vec::new();
            let mut child = self.node(node).first_child;
            while child != NONE {
                kids.push(child);
                child = self.node(child).next_sibling;
            }
            for &child in kids.iter().rev() {
                walk.push(child);
            }
        }
        None
    }

    pub(crate) fn last_name(&self, id: u32) -> Option<String> {
        let mut last = None;
        let mut walk = vec![id];
        while let Some(node) = walk.pop() {
            let kind = self.kind(node);
            if matches!(kind, "identifier" | "property_identifier" | "type_identifier") {
                last = Some(self.text_string(node));
            }
            // Depth-first in source order, so the last name assigned is the
            // rightmost one: `func.placeholder` is `placeholder`. Pushing the
            // children left to right popped them right to left and left the
            // caller with `func`.
            let mut kids = Vec::new();
            let mut child = self.node(node).first_child;
            while child != NONE {
                kids.push(child);
                child = self.node(child).next_sibling;
            }
            for &child in kids.iter().rev() {
                walk.push(child);
            }
        }
        last
    }

    /// `def f(a, b=1, *c)`, `function f(a, {b}, ...c)`, `(a, b) => {}`,
    /// `func f(id int)`, `Item find(int id)`.
    pub(crate) fn params(&self, def: u32, list: u32) -> Vec<String> {
        let mut out = Vec::new();
        let list_kind = self.kind(list);
        // JS class heritage shares the slot but is not a parameter list.
        if list_kind == "class_heritage" {
            return out;
        }
        // Java and C-family declarations are `type name`, so the name is the
        // last identifier of the parameter, not the first (which is the type).
        let type_first = self.kinds.family == Family::Java;
        for child in self.children(list) {
            let kind = self.kind(child);
            match kind {
                "(" | ")" | "," | "[" | "]" | "{" | "}" | ":" => {}
                "parameter" | "identifier" | "required_parameter" | "optional_parameter" | "rest_parameter"
                | "typed_parameter" | "default_parameter" | "typed_default_parameter" | "list_splat_pattern"
                | "dictionary_splat_pattern" => {
                    if let Some(name) = self.parameter_name(child, type_first) {
                        out.push(name);
                    }
                }
                "object_pattern" | "array_pattern" => {
                    out.extend(self.pattern_names(child));
                }
                _ => {
                    if let Some(name) = self.parameter_name(child, type_first) {
                        out.push(name);
                    }
                }
            }
        }
        let _ = def;
        out
    }

    /// The value of one enum member as a flow form: a literal is `0`, and a
    /// reference is its name - the same shapes the lexer writes.
    pub(crate) fn enum_member_value(&self, id: u32) -> String {
        for child in self.children(id) {
            let kind = self.kind(child);
            if matches!(kind, "identifier") {
                let text = self.text_string(child);
                let mut form = String::from("[\"n\",");
                push_string(&mut form, &text);
                form.push(']');
                return form;
            }
            if kind.contains("literal") || kind == "number" || kind == "integer" {
                return "0".to_string();
            }
        }
        "0".to_string()
    }

    pub(crate) fn first_identifier(&self, id: u32) -> Option<String> {
        if matches!(self.kind(id), "identifier" | "property_identifier")
            || self.kinds.soft.contains(&self.kind(id))
        {
            return Some(self.text_string(id));
        }
        let mut child = self.node(id).first_child;
        while child != NONE {
            if let Some(name) = self.first_identifier(child) {
                return Some(name);
            }
            child = self.node(child).next_sibling;
        }
        None
    }

    /// The parameter's own name: the first identifier of a `name type` list
    /// (Go) and the last of a `type name` one (Java, `List<Item> items`).
    pub(crate) fn parameter_name(&self, id: u32, type_first: bool) -> Option<String> {
        if !type_first {
            return self.first_identifier(id);
        }
        let mut leaves = Vec::new();
        self.collect_leaves(id, &mut leaves);
        leaves
            .into_iter()
            .filter(|&leaf| matches!(self.kind(leaf), "identifier"))
            .next_back()
            .map(|leaf| self.text_string(leaf))
    }

}

/// Byte offset of the start of each line, so a node's line is a binary search.
fn line_starts(src: &[u8]) -> Vec<u32> {
    let mut out = vec![0u32];
    for (index, byte) in src.iter().enumerate() {
        if *byte == b'\n' {
            out.push(index as u32 + 1);
        }
    }
    out
}

#[cfg(test)]
mod accessor_name_tests {
    use super::*;
    use crate::deep_integration::{self, Lang};

    /// `get` and `set` are the words a grammar uses for accessors, so a
    /// function that carries one as its own name arrives with that anonymous
    /// token instead of an `identifier`, and the walk named nothing: the lexer
    /// had `set` and the CST had `<anonymous>`, which is a fallback on every
    /// `module.exports` helper called `set` in the wild.
    #[test]
    fn a_function_named_set_or_get_still_has_a_name() {
        for (source, expected) in [
            ("function set(a, b) { return a; }", "set"),
            ("function get(a) { return a; }", "get"),
            ("function setter(a) { return a; }", "setter"),
            ("return function async(_x) {};", "async"),
        ] {
            let tree = deep_integration::parse_file(source.as_bytes(), Lang::JavaScript).expect("a tree");
            let facts = extract(source.as_bytes(), &tree, Lang::JavaScript);
            let names: Vec<&str> = facts.symbols.iter().map(|symbol| symbol.name.as_str()).collect();
            assert!(names.contains(&expected), "{source} named {names:?}");
        }
    }

    #[test]
    fn a_bound_function_expression_takes_the_declarator_name() {
        let source = "const noopfn = function() {\n    ;\n}.bind();\n";
        let tree = deep_integration::parse_file(source.as_bytes(), Lang::JavaScript).expect("a tree");
        let facts = extract(source.as_bytes(), &tree, Lang::JavaScript);
        let names: Vec<&str> = facts.symbols.iter().map(|symbol| symbol.name.as_str()).collect();
        assert!(names.contains(&"noopfn"), "{source} named {names:?}");
        assert!(!names.iter().any(|name| name.starts_with("<anonymous>")), "{source} named {names:?}");
    }
}
