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
//! TypeScript, TSX, Rust, Go, Java, C and C++ have tables, and Rust's and the
//! C family's walk arms live in their own regions below. JSON has empty tables
//! on purpose: a data file's only fact is the module symbol, so [`lang_for`]
//! answers `None` for it and the lexer path is kept. C and C++ also answer
//! `None` from `lang_for`: their tables feed `tests/facts_c_cpp.rs` and the
//! `--compare` audit, but the differential gate does not yet prefer the CST
//! record for enough real files. The other grammars fall back to the module
//! symbol until their table is written.
//!
//! Requires the `deep` and `parser` features: the Cst comes from
//! `deep_integration`, which names the runtime's kind table, and the runtime
//! (`src/deep`) is what the `parser` feature compiles.

#![cfg(feature = "deep")]

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
    Def { kind: "class_definition", name: &["identifier"], params: Some("argument_list"), params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "function_definition", name: &["identifier"], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];

const JS_DEFS: &[Def] = &[
    Def { kind: "class_declaration", name: &["identifier"], params: Some("class_heritage"), params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "class", name: &["identifier"], params: Some("class_heritage"), params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "interface_declaration", name: &["type_identifier", "identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "enum_declaration", name: &["identifier", "type_identifier"], params: None, params_nth: 0, name_last: false, receiver: false, container: true },
    Def { kind: "function_declaration", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "generator_function_declaration", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "method_definition", name: &["property_identifier", "identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "function_expression", name: &["identifier"], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
    Def { kind: "arrow_function", name: &[], params: Some("formal_parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
];

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
    Def { kind: "function_item", name: &["identifier", "default"], params: Some("parameters"), params_nth: 0, name_last: false, receiver: false, container: false },
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
/// [`Walker::c_imports`] reads the directive lines from the source.
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

fn kinds(lang: Lang) -> Kinds {
    match lang {
        Lang::Python => {
            Kinds { definitions: PY_DEFS, calls: PY_CALLS, imports: PY_IMPORTS, soft: PY_SOFT, family: Family::Plain }
        }
        Lang::C | Lang::Cpp => Kinds {
            definitions: if lang == Lang::Cpp { CPP_DEFS } else { C_DEFS },
            calls: if lang == Lang::Cpp { CPP_CALLS } else { C_CALLS },
            imports: if lang == Lang::Cpp { CPP_IMPORTS } else { C_IMPORTS },
            soft: &[],
            family: Family::Plain,
        },
        Lang::Rust => {
            Kinds { definitions: RS_DEFS, calls: RS_CALLS, imports: RS_IMPORTS, soft: &[], family: Family::Rust }
        }
        Lang::Json => {
            Kinds { definitions: JSON_DEFS, calls: JSON_CALLS, imports: JSON_IMPORTS, soft: &[], family: Family::Plain }
        }
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
            Kinds { definitions: JS_DEFS, calls: JS_CALLS, imports: JS_IMPORTS, soft: &[], family: Family::Js }
        }
        // Go and Java write their statements and their calls themselves: the
        // lexer's statement model is mirrored by `decl_value` rather than by
        // `form`, so the generic arms below are not reached.
        Lang::Go => Kinds { definitions: GO_DEFS, calls: &[], imports: GO_IMPORTS, soft: &[], family: Family::Go },
        Lang::Java => Kinds { definitions: JAVA_DEFS, calls: &[], imports: JAVA_IMPORTS, soft: &[], family: Family::Java },
        // The other grammars have no definition table here yet; the walk still
        // returns a record with the module symbol, which is what the caller
        // falls back on. Named so a caller can tell "no facts" from "empty file".
        _ => Kinds { definitions: &[], calls: &[], imports: &[], soft: &[], family: Family::Plain },
    }
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
    fn new(src: &'a [u8], tree: &'a Cst, lang: Lang) -> Self {
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
        }
    }

    fn run(&mut self) {
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
    fn kind(&self, id: u32) -> &'static str {
        self.language.name(self.nodes[id as usize].kind)
    }

    #[inline]
    fn node(&self, id: u32) -> Node {
        self.nodes[id as usize]
    }

    /// The text of a leaf the grammar spells as a soft keyword. It is a name
    /// wherever the mapping reads a name: `match.group(1)`, `type(x)`,
    /// `case = ...`. Anywhere else the same kind is unreachable, because the
    /// statement kinds (`match_statement`, `case_clause`) are nodes, not leaves.
    fn soft_name(&self, id: u32) -> Option<String> {
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
    fn text(&self, id: u32) -> &'a [u8] {
        let node = self.node(id);
        &self.src[node.start as usize..node.end as usize]
    }

    fn text_string(&self, id: u32) -> String {
        String::from_utf8_lossy(self.text(id)).into_owned()
    }

    fn line_of(&self, offset: u32) -> u32 {
        match self.line_starts.binary_search(&offset) {
            Ok(index) => index as u32 + 1,
            Err(index) => index as u32,
        }
    }

    #[inline]
    fn line(&self, id: u32) -> u32 {
        self.line_of(self.node(id).start)
    }

    fn end_line(&self, id: u32) -> u32 {
        self.line_of(self.node(id).end.saturating_sub(1))
    }

    fn children(&self, id: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut child = self.node(id).first_child;
        while child != NONE {
            out.push(child);
            child = self.node(child).next_sibling;
        }
        out
    }

    fn child_of_kind(&self, id: u32, kind: &str) -> Option<u32> {
        self.nth_child_of_kind(id, kind, 0)
    }

    /// The `nth` child of a kind, in order: Go writes a method's receiver list
    /// before its parameter list, so the list a definition wants is the second.
    fn nth_child_of_kind(&self, id: u32, kind: &str, nth: usize) -> Option<u32> {
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
    fn leaf_name(&self, id: u32) -> Option<String> {
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

    fn has_kind(&self, id: u32, kinds: &[&str]) -> bool {
        kinds.contains(&self.kind(id))
    }

    /// The first child that is neither punctuation nor a keyword we skip.
    fn expression_child(&self, id: u32) -> Option<u32> {
        self.children(id).into_iter().find(|&child| !self.is_layout(child))
    }

    /// Punctuation and keywords: nodes with no children whose text is not a
    /// name. They never carry a value of their own.
    fn is_layout(&self, id: u32) -> bool {
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

    fn frame(&self) -> &Frame {
        self.stack.last().expect("the module frame is never popped")
    }

    fn add_symbol(&mut self, name: String, owner: String, kind: u8, line: u32, params: Vec<String>) -> usize {
        let base = if owner.is_empty() { name.clone() } else { format!("{owner}.{name}") };
        let taken = self.facts.symbols.iter().any(|symbol| symbol.key == base);
        let key = if taken { format!("{base}@{line}") } else { base };
        self.facts.symbols.push(Symbol { key, name, owner, kind, line, end_line: line, params, ..Symbol::default() });
        self.facts.symbols.len() - 1
    }

    fn event(&mut self, text: &str) {
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
    fn named_call(&self, id: u32) -> bool {
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

    fn push_call(&mut self, path: String, line: u32) -> usize {
        let symbol = self.frame().symbol;
        let calls = &mut self.facts.symbols[symbol].calls;
        calls.push(Call { path, line });
        calls.len() - 1
    }

    // -- enter / leave ------------------------------------------------------

    fn enter(&mut self, id: u32) {
        let kind = self.kind(id);
        if self.is_rust() && self.enter_rust(id) {
            return;
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
            "expression_statement" => self.expression_statement(id),
            "return_statement" => self.return_statement(id),
            "variable_declarator" if self.is_js() => self.declarator(id),
            "public_field_definition" | "field_definition" | "property_signature" if self.is_js() => {
                self.field(id)
            }
            "throw_statement" | "decorator" => self.expression_statement(id),
            // `raise E(x) from c(y)`, `assert a(v), b(v)`, `del xs[k(v)]`:
            // every value-bearing child is read, not only the first.
            "raise_statement" | "assert_statement" | "delete_statement" => self.statement_values(id),
            // `pass`, `break`, `continue`: the one-pass lexer reads the word as
            // an expression and files an `x` event for it, so the CST does too.
            // The module symbol is emitted only when it has events, so losing
            // the event for a module-level `pass` dropped the whole symbol.
            "pass_statement" | "break_statement" | "continue_statement" => {
                let line = self.line(id);
                let word = self.text_string(id);
                let mut text = format!("[\"x\",{line},[\"n\",");
                push_string(&mut text, &word);
                text.push_str("]]");
                self.event(&text);
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
                }
                for child in self.children(id) {
                    if Some(child) == bound_value
                        || self.is_layout(child)
                        || matches!(self.kind(child), "block" | "statement_block")
                    {
                        continue;
                    }
                    self.form(child);
                }
            }
            "for_statement" | "for_in_statement" => {
                self.event("[\"b\"]");
                if let Some((target, value)) = self.loop_binding(id) {
                    self.assignment_event(target, value, self.line(id), false);
                }
            }
            "while_statement" => {
                self.condition_before(id);
                self.event("[\"b\"]");
            }
            "try_statement" | "switch_statement" | "match_statement" => self.event("[\"b\"]"),
            "catch_clause" | "finally_clause" | "except_clause" | "switch_case" | "case_clause" => {
                self.event("[\"e\"]")
            }
            _ => {}
        }
    }

    fn leave(&mut self, id: u32) {
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
                | "try_statement"
                | "switch_statement"
                | "match_statement"
                | "with_statement"
        ) {
            self.event("[\"j\"]");
        }
        if self.frame().node != id {
            return;
        }
        let frame = self.stack.pop().expect("a frame to pop");
        if let Some(index) = frame.defines {
            self.facts.symbols[index].end_line = self.end_line(id);
        }
    }

    fn is_js(&self) -> bool {
        self.kinds.family == Family::Js
    }

    /// `true` for Rust: `impl` names itself from its type, `use` is a path
    /// import, and the flow events are expressions, not statements.
    fn is_rust(&self) -> bool {
        self.kinds.family == Family::Rust
    }

    // -- definitions --------------------------------------------------------

    fn open_definition(&mut self, id: u32, def: &Def) {
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
            return;
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
            let at = self.line(lambda);
            let value = self.form(lambda);
            if value != "0" {
                self.event(&format!("[\"r\",{at},{value}]"));
            }
        }
    }

    /// The name a definition node carries, if any.
    fn def_name(&self, id: u32, def: &Def) -> Option<String> {
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
        // A method called `get` or `set` reads as the modifier that introduces
        // a getter, so the grammar gives it that kind: the name is then the leaf
        // just before the parameter list.
        if def.kind == "method_definition" {
            let children = self.children(id);
            let at = children.iter().position(|&child| self.kind(child) == "formal_parameters")?;
            let name = children.get(at.checked_sub(1)?)?;
            if self.node(*name).first_child == NONE {
                return Some(self.text_string(*name));
            }
        }
        None
    }

    fn bases(&self, id: u32) -> Vec<String> {
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

    fn last_name(&self, id: u32) -> Option<String> {
        let mut last = None;
        let mut walk = vec![id];
        while let Some(node) = walk.pop() {
            let kind = self.kind(node);
            if matches!(kind, "identifier" | "property_identifier" | "type_identifier") {
                last = Some(self.text_string(node));
            }
            let mut child = self.node(node).first_child;
            while child != NONE {
                walk.push(child);
                child = self.node(child).next_sibling;
            }
        }
        last
    }

    /// `def f(a, b=1, *c)`, `function f(a, {b}, ...c)`, `(a, b) => {}`,
    /// `func f(id int)`, `Item find(int id)`.
    fn params(&self, def: u32, list: u32) -> Vec<String> {
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

    fn first_identifier(&self, id: u32) -> Option<String> {
        if matches!(self.kind(id), "identifier" | "property_identifier") {
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
    fn parameter_name(&self, id: u32, type_first: bool) -> Option<String> {
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

    // -- Go and Java --------------------------------------------------------
    //
    // The one-pass lexer reads these two languages as statements over tokens:
    // a `{` after a name opens a block, a `;` (or, in Go, a line end) ends a
    // statement, and every statement is an `a`/`r`/`x` event whose value is the
    // names and calls in it. The tree says exactly which statements exist, so
    // the arms below write the same events from the nodes rather than from a
    // token run. `decl_value` is the value side: the names the lexer would read
    // and the calls it would record, in the same shapes.

    fn go_enter(&mut self, id: u32) {
        match self.kind(id) {
            // Top-level declarations are statements to the lexer, exactly as a
            // block's are: `var version = "0.1.0"` is an assignment on the
            // module symbol.
            "source_file" => self.go_statements(id),
            // `package shop`, `type Repo struct {...}`: a declaration the lexer
            // keeps as a statement. Its value is the whole header, so a field
            // name in it is a name the lexer read on that line too.
            "package_clause" | "type_declaration" => self.decl_statement(id),
            // A struct field is its own statement to the lexer, because `struct
            // {` opened a block: `Name string` sits on the module symbol.
            "field_declaration" => self.decl_statement(id),
            // An interface method is a statement there too: `Get(id int) (...)`.
            "method_elem" => self.decl_statement(id),
            // `~int | ~int64` in a generic constraint is a statement of its own.
            "type_elem" => self.decl_statement(id),
            // `[]string{"GET /"}`: the `{` after `]string` opens a block to the
            // lexer, so the literal's elements are a statement on their own.
            "literal_value" => self.go_literal(id),
            "block" => self.go_statements(id),
            "for_statement" => match self.child_of_kind(id, "range_clause") {
                Some(_) => self.go_range_loop(id),
                None => self.go_header(id),
            },
            "if_statement" | "expression_switch_statement" | "type_switch_statement" | "switch_statement"
            | "select_statement" => self.go_header(id),
            "expression_case" | "default_case" | "type_case" | "communication_case" => self.go_case(id),
            _ => {}
        }
    }

    fn java_enter(&mut self, id: u32) {
        match self.kind(id) {
            // `package com.example.web;`, `open module a.b {`: the lexer reads
            // the dotted name as one name, so the value is one name too.
            "package_declaration" | "module_declaration" | "requires_module_directive"
            | "exports_module_directive" | "opens_module_directive" | "uses_module_directive"
            | "provides_module_directive" => self.java_dotted_declaration(id),
            "block" | "class_body" | "enum_body" | "interface_body" | "constructor_body" | "module_body" => {
                self.java_statements(id)
            }
            "if_statement" | "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement"
            | "switch_expression" | "switch_statement" => self.java_header(id),
            "switch_block_statement_group" | "switch_rule" => self.java_case(id),
            // `@RestController public class Controller {`: with an annotation in
            // front of it the lexer does not see a class at all and reads the
            // header as an expression, so the header is an `x` event here too.
            "class_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration"
            | "annotation_type_declaration" => self.java_header_event(id),
            _ => {}
        }
    }

    /// The declaration's header (everything before its body) as one expression
    /// statement, which is what the lexer writes when it does not recognize the
    /// declaration as a class.
    fn java_header_event(&mut self, id: u32) {
        let mut parts = Vec::new();
        for child in self.children(id) {
            if matches!(
                self.kind(child),
                "class_body" | "enum_body" | "interface_body" | "annotation_type_body" | "record_body" | "constructor_body"
            ) {
                continue;
            }
            let value = self.decl_value(child);
            if !value.is_empty() && value != "0" {
                parts.push(value);
            }
        }
        let value = match parts.len() {
            0 => return,
            1 => parts.pop().unwrap_or_default(),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        };
        let line = self.line(id);
        self.lexer_event(b'x', line, &[], &value, false);
    }

    /// A declaration whose name is dotted (`com.example.web`). The lexer joins
    /// the segments, so the value carries the joined name and each keyword
    /// before it as its own name.
    fn java_dotted_declaration(&mut self, id: u32) {
        let mut parts = Vec::new();
        let mut path = String::new();
        let flush = |path: &mut String, parts: &mut Vec<String>| {
            if path.is_empty() {
                return;
            }
            let mut name = "[\"n\",".to_string();
            push_string(&mut name, path);
            name.push(']');
            parts.push(name);
            path.clear();
        };
        let children = self.children(id);
        for (index, &child) in children.iter().enumerate() {
            // A name is part of a dotted path when a dot sits on either side of
            // it; a lone word (`exports`, `requires`) is a name of its own.
            let dotted = children.get(index.wrapping_sub(1)).is_some_and(|&before| self.kind(before) == ".")
                || children.get(index + 1).is_some_and(|&after| self.kind(after) == ".");
            if let Some(name) = self.leaf_name(child).filter(|_| dotted) {
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(&name);
                continue;
            }
            if self.kind(child) == "." {
                continue;
            }
            // `exports a.b to c.d`: the keyword between the two paths ends the
            // first one, so the path is not run together across it.
            flush(&mut path, &mut parts);
            let value = self.decl_value(child);
            if !value.is_empty() && value != "0" {
                parts.push(value);
            }
        }
        flush(&mut path, &mut parts);
        let value = match parts.len() {
            0 => return,
            1 => parts.pop().unwrap_or_default(),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        };
        let line = self.line(id);
        self.lexer_event(b'x', line, &[], &value, false);
    }

    /// One statement-shaped node, as the lexer writes it: an assignment when
    /// there is a binder, a return when the word is `return`, an expression
    /// otherwise.
    fn decl_statement(&mut self, id: u32) {
        let value = self.decl_value(id);
        let line = self.line(id);
        self.lexer_event(b'x', line, &[], &value, false);
    }

    fn go_statements(&mut self, block: u32) {
        for child in self.children(block) {
            self.go_statement(child);
        }
    }

    fn java_statements(&mut self, body: u32) {
        for child in self.children(body) {
            self.java_statement(child);
        }
    }

    /// Emit the event `id` would produce as a statement, if it is one. Returns
    /// whether it was: a header calls this for its init statement and falls
    /// back to a plain value for anything else.
    fn go_statement(&mut self, id: u32) -> bool {
        match self.kind(id) {
            "short_var_declaration" | "assignment_statement" => self.go_assign(id),
            "return_statement" => self.go_return(id),
            "var_declaration" | "const_declaration" => self.go_var(id),
            "call_expression" | "defer_statement" | "go_statement" | "inc_statement" | "send_statement" => {
                self.decl_statement(id)
            }
            _ => return false,
        }
        true
    }

    fn java_statement(&mut self, id: u32) {
        match self.kind(id) {
            "local_variable_declaration" | "field_declaration" => self.java_declaration(id),
            "constant_declaration" => self.java_declaration(id),
            "expression_statement" => match self.expression_child(id) {
                Some(child) if self.kind(child) == "assignment_expression" => self.java_assign(child),
                Some(child) => {
                    let value = self.decl_value(id);
                    let line = self.line(id);
                    self.lexer_event(b'x', line, &[], &value, false);
                    let _ = child;
                }
                None => {}
            },
            "return_statement" => self.java_return(id),
            // `throw new X()`, `assert a == b`, `break`, `continue`, `yield v`:
            // the lexer reads the statement as one expression, so the event is
            // an `x` whose value is the names and calls in it.
            "requires_module_directive" | "exports_module_directive" | "opens_module_directive"
            | "uses_module_directive" | "provides_module_directive" => self.java_dotted_declaration(id),
            "throw_statement" | "assert_statement" | "break_statement" | "continue_statement" | "yield_statement"
            | "labeled_statement" | "synchronized_statement" => {
                let value = self.decl_value(id);
                let line = self.line(id);
                self.lexer_event(b'x', line, &[], &value, false);
            }
            "try_statement" => {
                self.java_statements(id);
            }
            _ => {}
        }
    }

    /// `int total = 0;`, `List<Item> items = f();`, `private Map<...> index;`.
    /// With an initializer the lexer writes an assignment to the declared name;
    /// without one the statement is an expression, and the type names are in it.
    fn java_declaration(&mut self, id: u32) {
        let children = self.children(id);
        let Some(declarator) = children.iter().copied().find(|&child| self.kind(child) == "variable_declarator")
        else {
            return self.decl_statement(id);
        };
        let parts = self.children(declarator);
        let Some(equal) = parts.iter().position(|&child| self.kind(child) == "=") else {
            return self.decl_statement(id);
        };
        if let Some(class) = parts.get(equal + 1).and_then(|&value| self.constructed_class(value)) {
            if let Some(target) = parts.first().and_then(|&name| self.decl_target(name)) {
                self.facts.vars.push((target, class));
            }
        }
        let value = self.decl_values(&parts[equal + 1..]);
        let targets = self.java_targets(id);
        let line = self.line(id);
        self.lexer_event(b'a', line, &targets, &value, false);
    }

    /// The names a Java declaration binds, read the way the lexer reads them:
    /// every leaf before the initializer's `=`, split at commas, and the last
    /// name of each part. The commas inside a generic type split too, because
    /// the lexer's unpacker counts only brackets: `Map<K, List<Item>> items`
    /// binds `K` and `items` to it.
    fn java_targets(&self, id: u32) -> Vec<String> {
        let mut parts: Vec<Vec<String>> = vec![Vec::new()];
        let mut leaves = Vec::new();
        self.collect_leaves(id, &mut leaves);
        for leaf in leaves {
            let text = self.text_string(leaf);
            if text == "=" {
                break;
            }
            if text == "," {
                parts.push(Vec::new());
                continue;
            }
            let bytes = text.as_bytes();
            if bytes.first().is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
                && bytes.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                if let Some(last) = parts.last_mut() {
                    last.push(text);
                }
            }
        }
        parts.into_iter().filter_map(|part| part.into_iter().next_back()).collect()
    }

    /// Every leaf of a node, in source order.
    fn collect_leaves(&self, id: u32, out: &mut Vec<u32>) {
        let mut pending = vec![id];
        let mut order = Vec::new();
        while let Some(node) = pending.pop() {
            let mut child = self.node(node).first_child;
            let mut kids = Vec::new();
            while child != NONE {
                kids.push(child);
                child = self.node(child).next_sibling;
            }
            if kids.is_empty() {
                order.push(node);
            } else {
                for &kid in kids.iter().rev() {
                    pending.push(kid);
                }
            }
        }
        out.extend(order);
    }

    /// `for _, v := range xs { ... }`: the lexer's `{` after the clause is not a
    /// block (the clause holds a top-level assignment), so the loop is one
    /// statement and the event's value carries the body's names too.
    fn go_range_loop(&mut self, id: u32) {
        let mut targets = Vec::new();
        if let Some(clause) = self.child_of_kind(id, "range_clause") {
            for child in self.children(clause) {
                if matches!(self.text_string(child).as_str(), ":=" | "=") {
                    break;
                }
                if self.is_layout(child) {
                    continue;
                }
                if self.kind(child) == "expression_list" {
                    for part in self.children(child) {
                        if let Some(target) = self.decl_target(part) {
                            targets.push(target);
                        }
                    }
                } else if let Some(target) = self.decl_target(child) {
                    targets.push(target);
                }
            }
        }
        let value = self.decl_value(id);
        let line = self.line(id);
        self.lexer_event(b'a', line, &targets, &value, false);
        self.event("[\"b\"]");
    }

    /// `a, b := f()`, `x = y`, `xs[i] += 1`.
    fn go_assign(&mut self, id: u32) {
        let children = self.children(id);
        let split = children.iter().position(|&child| {
            self.node(child).first_child == NONE
                && matches!(self.text_string(child).as_str(), "=" | ":=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=")
        });
        let Some(at) = split else { return };
        let operator = self.text_string(children[at]);
        let targets: Vec<String> = children[..at]
            .iter()
            .filter(|&&child| !self.is_layout(child))
            .filter_map(|&child| self.decl_target(child))
            .collect();
        if targets.len() == 1 {
            if let Some(class) = children.get(at + 1).and_then(|&value| self.constructed_class(value)) {
                self.facts.vars.push((targets[0].clone(), class));
            }
        }
        let value = self.decl_values(&children[at + 1..]);
        let line = self.line(id);
        let weak = !matches!(operator.as_str(), "=" | ":=");
        self.lexer_event(b'a', line, &targets, &value, weak);
    }

    fn java_assign(&mut self, id: u32) {
        let children = self.children(id);
        let split = children.iter().position(|&child| {
            self.node(child).first_child == NONE
                && matches!(self.text_string(child).as_str(), "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=")
        });
        let Some(at) = split else {
            let value = self.decl_value(id);
            let line = self.line(id);
            self.lexer_event(b'x', line, &[], &value, false);
            return;
        };
        let operator = self.text_string(children[at]);
        let targets: Vec<String> = children[..at]
            .iter()
            .filter(|&&child| !self.is_layout(child))
            .filter_map(|&child| self.decl_target(child))
            .collect();
        let value = self.decl_values(&children[at + 1..]);
        let line = self.line(id);
        self.lexer_event(b'a', line, &targets, &value, operator != "=");
    }

    fn go_return(&mut self, id: u32) {
        let children = self.children(id);
        let rest: Vec<u32> = children.into_iter().skip(1).collect();
        let value = self.decl_values(&rest);
        let line = self.line(id);
        self.lexer_event(b'r', line, &[], &value, false);
    }

    fn java_return(&mut self, id: u32) {
        let children = self.children(id);
        let rest: Vec<u32> = children.into_iter().skip(1).collect();
        let value = self.decl_values(&rest);
        let line = self.line(id);
        self.lexer_event(b'r', line, &[], &value, false);
    }

    /// `var zero V` is an expression to the lexer; `var x = f()` is an
    /// assignment to `x`; a `var (...)` group is one expression statement.
    fn go_var(&mut self, id: u32) {
        let children = self.children(id);
        if children.iter().any(|&child| self.kind(child) == "var_spec_list") {
            return self.decl_statement(id);
        }
        let Some(spec) = children.iter().copied().find(|&child| matches!(self.kind(child), "var_spec" | "const_spec"))
        else {
            return self.decl_statement(id);
        };
        let parts = self.children(spec);
        let Some(equal) = parts.iter().position(|&child| self.kind(child) == "=") else {
            return self.decl_statement(id);
        };
        let target = parts.first().and_then(|&name| self.decl_target(name)).into_iter().collect::<Vec<_>>();
        let value = self.decl_values(&parts[equal + 1..]);
        let line = self.line(id);
        self.lexer_event(b'a', line, &target, &value, false);
    }

    /// `if cond {`, `for i := 0; i < n; i++ {`, `switch x {`: every statement the
    /// lexer writes before the body is written here, one event each, and a
    /// branch marker for the body.
    fn go_header(&mut self, id: u32) {
        for child in self.children(id) {
            if self.is_layout(child) {
                continue;
            }
            let kind = self.kind(child);
            if matches!(kind, "block" | "else_clause" | "literal_value" | "field_declaration_list") {
                continue;
            }
            if self.go_statement(child) {
                continue;
            }
            let value = self.decl_value(child);
            let line = self.line(child);
            self.lexer_event(b'x', line, &[], &value, false);
        }
        // `switch v := f(x); v {` and `if v := f(x); p(v) {`: the lexer writes
        // one condition event over the whole header as well, and a value that
        // holds the names of all of it covers that event.
        if let Some(value) = self.header_value(id) {
            let line = self.line(id);
            self.lexer_event(b'x', line, &[], &value, false);
        }
        self.event("[\"b\"]");
    }

    /// The header's values as one expression, or `None` when there is nothing
    /// but the block left.
    fn header_value(&mut self, id: u32) -> Option<String> {
        let mut parts = Vec::new();
        for child in self.children(id) {
            if self.is_layout(child) {
                continue;
            }
            let kind = self.kind(child);
            if matches!(
                kind,
                "block" | "else_clause" | "literal_value" | "field_declaration_list" | "switch_block"
                    | "catch_clause" | "finally_clause"
            ) {
                continue;
            }
            let value = self.decl_value(child);
            if !value.is_empty() && value != "0" {
                parts.push(value);
            }
        }
        match parts.len() {
            0 => None,
            1 => parts.pop(),
            _ => Some(format!("[\"m\",[{}]]", parts.join(","))),
        }
    }

    /// `[]string{"a", "b"}`: the elements, at the line of the first one, which
    /// is the line the lexer's statement starts on.
    fn go_literal(&mut self, id: u32) {
        let line = self
            .children(id)
            .into_iter()
            .find(|&child| !matches!(self.kind(child), "{" | "}" | ","))
            .map(|child| self.line(child))
            .unwrap_or_else(|| self.line(id));
        let value = self.decl_value(id);
        self.lexer_event(b'x', line, &[], &value, false);
    }

    /// Java keeps a header on one line and reads it as one expression, so the
    /// whole condition is one `x` event, however many values it holds.
    fn java_header(&mut self, id: u32) {
        let mut parts = Vec::new();
        for child in self.children(id) {
            if self.is_layout(child) {
                continue;
            }
            let kind = self.kind(child);
            if matches!(
                kind,
                "block" | "class_body" | "constructor_body" | "else_clause" | "catch_clause" | "finally_clause"
                    | "resource_specification" | "switch_block"
            ) {
                continue;
            }
            let value = self.decl_value(child);
            if !value.is_empty() && value != "0" {
                parts.push(value);
            }
        }
        let value = match parts.len() {
            0 => String::new(),
            1 => parts.pop().unwrap_or_default(),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        };
        if !value.is_empty() {
            let line = self.line(id);
            self.lexer_event(b'x', line, &[], &value, false);
        }
        self.event("[\"b\"]");
    }

    /// `case "x":`, `default:`. The lexer files the arm marker and then reads
    /// the statements after the colon.
    fn go_case(&mut self, id: u32) {
        self.event("[\"e\"]");
        let children = self.children(id);
        let colon = children.iter().position(|&child| {
            self.node(child).first_child == NONE && self.text(child) == b":"
        });
        let after = colon.map(|at| at + 1).unwrap_or(children.len());
        for child in children[after..].iter().copied() {
            if self.is_layout(child) {
                continue;
            }
            if self.go_statement(child) {
                continue;
            }
            let value = self.decl_value(child);
            let line = self.line(child);
            self.lexer_event(b'x', line, &[], &value, false);
        }
    }

    fn java_case(&mut self, id: u32) {
        self.event("[\"e\"]");
        for child in self.children(id) {
            self.java_statement(child);
        }
    }

    /// The value the lexer writes for a run of nodes: one merge of them, or the
    /// single value when there is only one.
    fn decl_values(&mut self, ids: &[u32]) -> String {
        let mut parts = Vec::new();
        for &id in ids {
            let value = self.decl_value(id);
            if !value.is_empty() {
                parts.push(value);
            }
        }
        match parts.len() {
            0 => "0".to_string(),
            1 => parts.pop().unwrap_or_else(|| "0".to_string()),
            // A run of values from a comma-separated list is a tuple to the
            // lexer, not a merge.
            _ => format!("[\"t\",[{}]]", parts.join(",")),
        }
    }

    /// The names the lexer reads in these nodes, as a flow value. Every leaf
    /// whose text is a word is a name (the lexer has no keyword table for these
    /// two grammars: `type`, `package` and `private` are names to it), a
    /// literal is `0`, and a call is one call in the symbol's table. A dotted
    /// name (`sync.Mutex`, `this.index`) is written as the path the lexer
    /// builds from it.
    fn decl_value(&mut self, id: u32) -> String {
        let node = self.node(id);
        if node.first_child == NONE {
            if node.start == node.end {
                return String::new();
            }
            let text = self.text(id);
            let first = text[0];
            if first.is_ascii_digit() || first == b'"' || first == b'\'' || first == b'`' {
                return "0".to_string();
            }
            if first.is_ascii_alphabetic() || first == b'_' {
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &String::from_utf8_lossy(text));
                out.push(']');
                return out;
            }
            return String::new();
        }
        let kind = self.kind(id);
        // A call is a call in the flow form the lexer writes: the index into
        // the symbol's call table, with the names and arguments beside it.
        // A call is the whole value: the lexer writes `["c",index,recv,args]`
        // and nothing beside it. Descending into the call's children as well
        // produced a merge of the callee's name and the raw arguments, which
        // never equals the lexer's event and made every Go file look like it had
        // lost a fact.
        if matches!(kind, "call_expression" | "method_invocation" | "object_creation_expression") {
            let index = self.decl_call(id);
            let args = self
                .children(id)
                .into_iter()
                .find(|&child| matches!(self.kind(child), "arguments" | "argument_list"))
                .map(|list| self.arguments(list))
                .unwrap_or_else(|| "[]".to_string());
            let callee = self.decl_callee(id);
            // `ts.Listener.Addr().String()`: the receiver is a call of its own,
            // and the lexer records it as one. Reading only the outer call lost
            // every chained call in Go (57 of the 96 remaining fallbacks on
            // net/http were exactly this shape).
            self.record_receiver_calls(callee);
            let path = self.decl_path(callee);
            let receiver = match path.rsplit_once('.') {
                Some((head, _)) if !head.is_empty() => {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, head);
                    out.push(']');
                    out
                }
                _ => "0".to_string(),
            };
            return format!("[\"c\",{index},{receiver},{args}]");
        }
        let mut parts = Vec::new();
        if matches!(
            kind,
            "selector_expression" | "qualified_type" | "field_access" | "dotted_name" | "scoped_identifier"
                | "scoped_type_identifier" | "nested_type_identifier" | "class_literal" | "method_reference"
        ) {
            let path = self.decl_path(id);
            if !path.is_empty() {
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &path);
                out.push(']');
                parts.push(out);
            }
        }
        for child in self.children(id) {
            let value = self.decl_value(child);
            if !value.is_empty() {
                parts.push(value);
            }
        }
        let comma = self.children(id).iter().any(|&child| self.kind(child) == ",");
        match parts.len() {
            0 => "0".to_string(),
            1 => parts.pop().unwrap_or_else(|| "0".to_string()),
            _ if comma => format!("[\"t\",[{}]]", parts.join(",")),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        }
    }

    /// One call, at the line its callee is written on, named the way the source
    /// spells it. Returns its index in the symbol's call table, which is what
    /// the flow form's `["c",id,...]` names.
    fn decl_call(&mut self, id: u32) -> usize {
        let callee = self.decl_callee(id);
        let path = self.decl_path(callee);
        let line = self.line(callee);
        self.push_call(path, line)
    }


    /// Record every call that sits in a callee's receiver chain.
    fn record_receiver_calls(&mut self, callee: u32) {
        let mut node = callee;
        for _ in 0..8 {
            if !matches!(self.kind(node), "selector_expression" | "member_expression" | "field_access") {
                return;
            }
            let Some(object) = self.children(node).into_iter().find(|&child| !self.is_layout(child)) else {
                return;
            };
            if matches!(self.kind(object), "call_expression" | "method_invocation") {
                self.decl_call(object);
                node = object;
                continue;
            }
            node = object;
        }
    }

    /// The callee of a call node: the expression before the argument list.
    fn decl_callee(&self, id: u32) -> u32 {
        let children = self.children(id);
        let kind = self.kind(id);
        if kind == "object_creation_expression" {
            return children
                .iter()
                .copied()
                .find(|&child| !matches!(self.kind(child), "new" | "arguments" | "type_arguments"))
                .unwrap_or(id);
        }
        let args = children
            .iter()
            .position(|&child| matches!(self.kind(child), "arguments" | "argument_list" | "type_arguments"));
        let end = args.unwrap_or(children.len());
        // The callee is read by its text, so a member this grammar gives a
        // keyword kind (`Mockito.when(x)`, `obj.record`) still has a name.
        children[..end]
            .iter()
            .copied()
            .rev()
            .find(|&child| {
                self.leaf_name(child).is_some()
                    || (!self.is_layout(child) && !matches!(self.kind(child), "type_arguments" | "type_parameter_list" | "index"))
            })
            .unwrap_or(id)
    }

    /// A dotted name built the way the lexer builds one, with the enclosing
    /// method's receiver spelled `self` as both paths spell it.
    fn decl_path(&self, id: u32) -> String {
        match self.kind(id) {
            "identifier" | "type_identifier" | "field_identifier" | "this" | "super" => {
                let text = self.text_string(id);
                let receiver = self.frame().receiver.as_str();
                if text == "this" || (!receiver.is_empty() && text == receiver) {
                    "self".to_string()
                } else {
                    text
                }
            }
            "selector_expression" | "qualified_type" | "field_access" | "scoped_identifier" | "dotted_name"
            | "scoped_type_identifier" | "nested_type_identifier" | "class_literal" | "method_reference" => {
                let children = self.children(id);
                // The last segment is read by its text: `Mockito.when` names a
                // member called `when`, which this grammar gives a keyword kind.
                let name = children
                    .iter()
                    .copied()
                    .rev()
                    .find(|&child| self.kind(child) != "." && self.leaf_name(child).is_some())
                    .map(|child| self.decl_path(child))
                    .unwrap_or_default();
                let base = children
                    .first()
                    .copied()
                    .filter(|&first| self.kind(first) != ".")
                    .map(|first| self.decl_path(first))
                    .unwrap_or_default();
                let base = if base.is_empty() { String::new() } else { format!("{base}.") };
                format!("{base}{name}")
            }
            "method_invocation" => {
                // A call has no name of its own: the name belongs to its callee.
                let callee = self.decl_callee(id);
                if callee == id {
                    return String::new();
                }
                self.decl_path(callee)
            }
            "call_expression" => {
                let callee = self.decl_callee(id);
                if callee == id {
                    return String::new();
                }
                self.decl_path(callee)
            }
            "index_expression" => self.children(id).first().map(|&base| self.decl_path(base)).unwrap_or_default(),
            // `[]byte(x)`, `*T`, `map[K]V`: a conversion names its type, and the
            // lexer reads the type's own name as the callee.
            "array_type" | "pointer_type" | "map_type" | "slice_type" | "channel_type" => self
                .last_name(id)
                .or_else(|| self.leaf_name(id))
                .unwrap_or_default(),
            "parenthesized_expression" | "unary_expression" | "type_assertion" => self
                .expression_child(id)
                .map(|child| self.decl_path(child))
                .unwrap_or_default(),
            "generic_type" => self.children(id).first().map(|&base| self.decl_path(base)).unwrap_or_default(),
            _ => self.leaf_name(id).unwrap_or_default(),
        }
    }

    /// The name an assignment target binds: `x`, `this.index`, `xs[0]` -> `xs`
    /// (`x, y := ...` splits into two children before this is called).
    fn decl_target(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "identifier" | "field_identifier" | "this" | "selector_expression" | "field_access" => {
                let path = self.decl_path(id);
                (!path.is_empty()).then_some(path)
            }
            "index_expression" => {
                let path = self.children(id).first().map(|&base| self.decl_path(base)).unwrap_or_default();
                (!path.is_empty()).then_some(path)
            }
            "parenthesized_expression" => self.expression_child(id).and_then(|child| self.decl_target(child)),
            _ => None,
        }
    }

    /// The class a value constructs, by the lexer's convention: a capitalised
    /// callee, a composite literal's type, or anything after `new`.
    fn constructed_class(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "object_creation_expression" => {
                let path = self.decl_path(self.decl_callee(id));
                let last = path.rsplit('.').next().unwrap_or("").to_string();
                last.starts_with(|ch: char| ch.is_uppercase()).then_some(last)
            }
            "call_expression" | "method_invocation" => {
                // The lexer's convention: a callee whose first *and* last names
                // are capitalised constructs that class (`Repo()`, `New()`); a
                // composite literal is not a call and binds nothing.
                let path = self.decl_path(id);
                let first = path.split('.').next().unwrap_or("");
                let last = path.rsplit('.').next().unwrap_or("").to_string();
                (first.starts_with(|ch: char| ch.is_uppercase())
                    && last.starts_with(|ch: char| ch.is_uppercase())
                    && !last.contains('('))
                .then_some(last)
            }
            _ => None,
        }
    }

    /// One flow event, written the way `crate::expr` documents it.
    fn lexer_event(&mut self, tag: u8, line: u32, targets: &[String], value: &str, weak: bool) {
        let text = match tag {
            b'a' => {
                let mut text = format!("[\"a\",{line},[");
                for (index, name) in targets.iter().enumerate() {
                    if index > 0 {
                        text.push(',');
                    }
                    push_string(&mut text, name);
                }
                text.push_str(&format!("],{value},{}]", u8::from(weak)));
                text
            }
            _ => format!("[\"{}\",{line},{value}]", tag as char),
        };
        self.event(&text);
    }

    /// `func (c *Cache[K, V]) Get`: the receiver's own name and its type's.
    fn go_receiver(&self, list: u32) -> (String, String) {
        let Some(param) = self.children(list).into_iter().find(|&child| self.kind(child) == "parameter_declaration")
        else {
            return (String::new(), String::new());
        };
        let children = self.children(param);
        let name = children
            .iter()
            .copied()
            .find(|&child| self.kind(child) == "identifier")
            .map(|child| self.text_string(child))
            .unwrap_or_default();
        let owner = children
            .iter()
            .copied()
            .rev()
            .find_map(|child| self.receiver_type(child))
            .unwrap_or_default();
        (name, owner)
    }

    /// The type a receiver declaration names, unwrapping a pointer and taking
    /// a generic's base name: `*Cache[K, V]` -> `Cache`.
    fn receiver_type(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "identifier" | "type_identifier" => Some(self.text_string(id)),
            "pointer_type" | "generic_type" => self.children(id).into_iter().find_map(|child| self.receiver_type(child)),
            "qualified_type" => self.last_name(id),
            _ => None,
        }
    }

    fn pattern_names(&self, id: u32) -> Vec<String> {
        let mut out = Vec::new();
        let mut walk = vec![id];
        while let Some(node) = walk.pop() {
            match self.kind(node) {
                "shorthand_property_identifier_pattern" | "shorthand_property_identifier" => {
                    out.push(self.text_string(node))
                }
                "pair" | "object_assignment_pattern" => {
                    if let Some(last) = self.children(node).into_iter().last() {
                        walk.push(last);
                    }
                }
                "identifier" | "property_identifier" | "private_property_identifier" => {
                    out.push(self.text_string(node).trim_start_matches('#').to_string())
                }
                _ => {
                    let mut child = self.node(node).first_child;
                    while child != NONE {
                        walk.push(child);
                        child = self.node(child).next_sibling;
                    }
                }
            }
        }
        out
    }

    // -- statements ---------------------------------------------------------

    fn expression_statement(&mut self, id: u32) {
        let Some(child) = self.expression_child(id) else { return };
        if let Some(module) = self.require_module(child) {
            let line = self.line(id);
            self.facts.imports.push(Import { line, module, kind: IMPORT_STAR, ..Import::default() });
            return;
        }
        match self.kind(child) {
            "assignment" | "assignment_expression" => self.assignment(child, false),
            // Rust's `=` is the same statement: the lexer records the target,
            // the value and the class the value constructs.
            _ if self.is_rust() && self.kind(child) == "assignment_expression" => {
                self.assignment(child, false)
            }
            "augmented_assignment" | "augmented_assignment_expression" => self.assignment(child, true),
            // `x: T = f()`: the targets are the names and the value is the last
            // child, as in an assignment. `x: T` alone binds nothing and has no
            // value, so only the annotated form that carries `=` is read.
            "annotated" => {
                if self.children(child).into_iter().any(|part| self.kind(part) == "=") {
                    self.assignment(child, false);
                } else {
                    // `rule_id: str` binds nothing, but the lexer reads the
                    // name and the annotation as one expression and files an
                    // event for it. Emitting nothing here lost that event in
                    // every dataclass-shaped class body.
                    let mut parts = Vec::new();
                    for part in self.children(child) {
                        if self.is_layout(part) || self.kind(part) == ":" {
                            continue;
                        }
                        let value = self.form(part);
                        if value != "0" {
                            parts.push(value);
                        }
                    }
                    if !parts.is_empty() {
                        let line = self.line(child);
                        self.event(&format!("[\"x\",{line},[\"m\",[{}]]]", parts.join(",")));
                    }
                }
            }
            _ => {
                let value = self.form(child);
                if value != "0" {
                    let line = self.line(child);
                    self.event(&format!("[\"x\",{line},{value}]"));
                }
            }
        }
    }

    /// A statement whose whole content is values: `raise`, `assert`, `del`.
    /// The keywords (`from`, `assert`, `del`) are layout and are skipped by
    /// [`Self::is_layout`]; each remaining child is an expression to read.
    fn statement_values(&mut self, id: u32) {
        for child in self.children(id) {
            if self.is_layout(child) {
                continue;
            }
            self.form(child);
        }
    }

    fn return_statement(&mut self, id: u32) {
        let line = self.line(id);
        let value = match self.expression_child(id) {
            Some(child) => self.form(child),
            None => "0".to_string(),
        };
        self.event(&format!("[\"r\",{line},{value}]"));
    }

    /// `const x = ...;` without an initializer is an expression, as the lexer
    /// has it; with one it is an assignment.
    /// `require('x')` is an import, not a call: the lexer path agrees, and a
    /// resolver treats the module string, not a callee named `require`.
    fn require_module(&self, call: u32) -> Option<String> {
        if !matches!(self.kind(call), "call_expression" | "call") {
            return None;
        }
        let children = self.children(call);
        let callee = children.first()?;
        if self.kind(*callee) != "identifier" || self.text_string(*callee) != "require" {
            return None;
        }
        let args = children.iter().find(|&&child| self.kind(child) == "arguments")?;
        let inner: Vec<u32> = self.children(*args).into_iter().filter(|&child| !self.is_layout(child)).collect();
        if inner.len() != 1 {
            return None;
        }
        (self.kind(inner[0]) == "string").then(|| self.string_value(inner[0]))
    }

    fn declarator(&mut self, id: u32) {
        let children = self.children(id);
        let Some(&name_node) = children.first() else { return };
        let equal = children.iter().position(|&child| self.kind(child) == "=");
        let Some(equal) = equal else {
            let line = self.line(id);
            let names = self.pattern_names(name_node);
            if names.len() == 1 {
                let mut text = format!("[\"x\",{line},[\"n\",");
                push_string(&mut text, &names[0]);
                text.push_str("]]");
                self.event(&text);
            }
            return;
        };
        let Some(&value) = children.get(equal + 1) else { return };
        if let Some(module) = self.require_module(value) {
            // `const { a, b } = require('m')` takes named members; `const x =
            // require('m')` binds the module itself, so it has no member name.
            let destructured = matches!(self.kind(name_node), "object_pattern" | "array_pattern");
            let names = self.pattern_names(name_node);
            let line = self.line(id);
            for name in names {
                let (member, alias) = if destructured { (name.clone(), name) } else { (String::new(), name) };
                self.facts
                    .imports
                    .push(Import { line, module: module.clone(), name: member, alias, kind: IMPORT_MODULE });
            }
            return;
        }
        // `const f = () => {}`: the binding is a definition, not a value, and
        // the walk opens the symbol when it reaches the function node.
        if self.kinds.definitions.iter().any(|def| def.kind == self.kind(value)) {
            let names = self.pattern_names(name_node);
            if let Some(name) = names.into_iter().next() {
                self.bound.push((value, name));
            }
            return;
        }
        let line = self.line(id);
        let targets = self.pattern_names(name_node);
        self.assignment_parts(targets, value, line, false);
    }

    /// A class field: `#store = new Map()` writes on the enclosing symbol, the
    /// way the lexer treats a statement in a class body.
    fn field(&mut self, id: u32) {
        let children = self.children(id);
        let Some(&name_node) = children.first() else { return };
        let Some(equal) = children.iter().position(|&child| self.kind(child) == "=") else { return };
        let Some(&value) = children.get(equal + 1) else { return };
        let targets = self.pattern_names(name_node);
        self.assignment_parts(targets, value, self.line(id), false);
    }

    fn assignment(&mut self, id: u32, augmented: bool) {
        let children = self.children(id);
        let Some(&first) = children.first() else { return };
        let Some(&last) = children.last() else { return };
        if first == last {
            return;
        }
        let line = self.line(id);
        let targets = self.assignment_targets(first);
        self.assignment_parts(targets, last, line, augmented);
    }

    fn assignment_targets(&self, id: u32) -> Vec<String> {
        match self.kind(id) {
            "pattern_list" | "tuple_pattern" | "array_pattern" | "list_pattern" | "tuple" | "list"
            | "expression_list" => {
                self.children(id).into_iter().filter_map(|child| self.value_path(child)).collect()
            }
            // The left side of a Python multi-target assignment (`a, b = g(x),
            // 1`) is a `pattern_list` here, and this arm maps it, so the
            // `["a",2,["a","b"],...]` event the lexer writes is written here
            // too. The arm has always listed the kind, but the note it carried
            // said the mapping was left out because `pipeline::loses` compared
            // event *counts* and so could not see the value the CST cannot read
            // on such a file's f-string return (`["r",3,0]` where the lexer
            // follows `["r",3,["n","a"]]`); with a count-only guard, mapping the
            // target would have kept that record. That condition is now met:
            // the guard compares targets and value shapes, so the same file
            // falls back on the value it lost. `tests/guard_truth.rs` checks
            // both halves — the target kind is `pattern_list` and both names
            // reach the record — and the full suite is green with the mapping
            // on. The `expression_list` in the arm above is the *value* form,
            // read by `form`, and adds nothing to a target.
            // `x: Dict[str, int] = {}`: the target is the name inside the
            // annotation, not the annotation. Reading it keeps the event an
            // assignment (`["a",...]`) instead of a bare expression (`["x",...]`).
            "annotated" => self.children(id).first().and_then(|&first| self.value_path(first)).into_iter().collect(),
            // `meter.exhausted = …`: a field path is a target like any other
            // path, and `value_path` does not read one.
            _ => match self.value_path(id) {
                Some(path) if !path.is_empty() => vec![path],
                _ => {
                    let path = self.path_of(id);
                    if path.is_empty() { Vec::new() } else { vec![path] }
                }
            },
        }
    }

    fn assignment_event(&mut self, targets: Vec<String>, value: u32, line: u32, augmented: bool) {
        self.assignment_parts(targets, value, line, augmented);
    }

    /// `provider = lambda client="": dashboard.payload(...)` is a definition,
    /// not a value: the one-pass lexer opens a symbol for it, so the CST walk
    /// does too, and its body is read inside that symbol so the calls land on
    /// it rather than on the enclosing function.
    fn bound_lambda(&mut self, name: &str, node: u32, line: u32) -> bool {
        if self.kind(node) != "lambda" {
            return false;
        }
        let owner = if self.frame().container { self.frame().class.clone() } else { String::new() };
        let params = self
            .child_of_kind(node, "lambda_parameters")
            .map(|list| self.params(node, list))
            .unwrap_or_default();
        let index = self.add_symbol(name.to_string(), owner.clone(), KIND_FUNCTION, line, params);
        let body = self.children(node).into_iter().rev().find(|&child| !self.is_layout(child));
        self.stack.push(Frame { node, symbol: index, class: owner, receiver: String::new(), container: false, defines: Some(index) });
        if let Some(body) = body {
            if !matches!(self.kind(body), "block" | "statement_block") {
                let at = self.line(body);
                let value = self.form(body);
                if value != "0" {
                    self.event(&format!("[\"r\",{at},{value}]"));
                }
            }
            self.facts.symbols[index].end_line = self.end_line(body);
        }
        self.stack.pop();
        true
    }

    fn assignment_parts(&mut self, targets: Vec<String>, value: u32, line: u32, augmented: bool) {
        if !augmented && targets.len() == 1 && self.bound_lambda(&targets[0].clone(), value, line) {
            return;
        }
        let weak = augmented || targets.is_empty();
        let expr = self.form(value);
        if targets.is_empty() {
            if expr != "0" {
                self.event(&format!("[\"x\",{line},{expr}]"));
            }
            return;
        }
        let mut text = format!("[\"a\",{line},[");
        for (index, name) in targets.iter().enumerate() {
            if index > 0 {
                text.push(',');
            }
            push_string(&mut text, name);
        }
        text.push_str(&format!("],{expr},{}]", u8::from(weak)));
        self.event(&text);
        if let Some(class) = self.constructed(value) {
            for target in targets {
                self.facts.vars.push((target, class.clone()));
            }
        }
    }

    /// The class a call constructs, by the same convention the lexer uses: a
    /// capitalised callee, or anything after `new`.
    fn constructed(&self, value: u32) -> Option<String> {
        // `Path(__file__).resolve().parent`: the value is an attribute chain
        // whose base is the call. The lexer reads the first token, so the
        // chain is unwrapped to the call that starts it.
        let call = self.value_root_call(value)?;
        if self.kind(call) == "new_expression" {
            let path = self.children(call).into_iter().find_map(|child| {
                matches!(self.kind(child), "identifier" | "member_expression" | "attribute").then(|| self.path_of(child))
            })?;
            let last = path.rsplit('.').next().unwrap_or(&path);
            return last.starts_with(|ch: char| ch.is_uppercase()).then_some(last.to_string());
        }
        let path = self.root_callee(call)?;
        let last = path.rsplit('.').next().unwrap_or("").to_string();
        (last.starts_with(|ch: char| ch.is_uppercase()) && !last.contains('(')).then_some(last)
    }

    /// The call a value starts with: `a().b` -> the `a()` node, `a.b()` ->
    /// `a.b()`, `(a)` -> the call in the parentheses.
    fn value_root_call(&self, id: u32) -> Option<u32> {
        match self.kind(id) {
            "call" | "call_expression" | "new_expression" => Some(id),
            "attribute" | "member_expression" | "subscript" | "subscript_expression" | "parenthesized_expression" => {
                self.children(id).first().and_then(|&object| self.value_root_call(object))
            }
            // `candidate = Path(x).parent / candidate`: a binary value starts
            // with its left operand, which is what the lexer's first token is.
            "binary_operator" | "binary_expression" | "boolean_operator" | "conditional_expression"
            | "ternary_expression" => self.children(id).first().and_then(|&left| self.value_root_call(left)),
            _ => None,
        }
    }

    /// The callee at the root of a call chain: `a.b().c()` -> `a.b`, `f()()` ->
    /// `f`, `a.b()` -> `a.b`.
    fn root_callee(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "call" | "call_expression" | "new_expression" => {
                let callee = self.children(id).into_iter().find(|&child| {
                    !matches!(self.kind(child), "new" | "argument_list" | "arguments" | "type_arguments" | "(" | ")")
                })?;
                (callee != id).then(|| self.root_callee(callee)).flatten()
            }
            "attribute" | "member_expression" => {
                let object = self.children(id).first().copied()?;
                match self.root_callee(object) {
                    Some(root) => Some(root),
                    None => {
                        let path = self.path_of(id);
                        (!path.is_empty()).then_some(path)
                    }
                }
            }
            _ => {
                let path = self.path_of(id);
                (!path.is_empty()).then_some(path)
            }
        }
    }

    // -- control flow -------------------------------------------------------

    fn condition_before(&mut self, id: u32) {
        // `if x:` / `while x:` / `async with x:`: the first child that is not a
        // keyword or punctuation is the test.
        for child in self.children(id) {
            if self.is_layout(child) {
                continue;
            }
            if matches!(self.kind(child), "block" | "statement_block" | "else_clause" | "elif_clause" | "catch_clause" | "finally_clause") {
                return;
            }
            let value = self.form(child);
            if value != "0" {
                let line = self.line(child);
                self.event(&format!("[\"x\",{line},{value}]"));
            }
            return;
        }
    }

    /// `for a, b in xs` / `for (const a of xs)` -> targets and iterable.
    fn loop_binding(&mut self, id: u32) -> Option<(Vec<String>, u32)> {
        let children = self.children(id);
        let mut split = None;
        for (index, &child) in children.iter().enumerate() {
            if matches!(self.kind(child), "in" | "of" | "as") {
                split = Some(index);
                break;
            }
        }
        // JS keeps the header inside `for_header`: `for (let x of y)`.
        if split.is_none() {
            let header = children.iter().copied().find(|&child| self.kind(child) == "for_header")?;
            let inner = self.children(header);
            let at = inner.iter().position(|&child| matches!(self.kind(child), "in" | "of"))?;
            let targets = inner[..at]
                .iter()
                .filter(|&&child| !matches!(self.kind(child), "(" | "let" | "const" | "var" | ";" | ","))
                .flat_map(|&child| {
                    if self.kind(child) == "variable_declarator" {
                        self.children(child).first().copied().map(|first| self.pattern_names(first)).unwrap_or_default()
                    } else {
                        self.pattern_names(child)
                    }
                })
                .collect();
            let value = *inner.get(at + 1)?;
            return Some((targets, value));
        }
        let at = split?;
        if self.kind(children[at]) == "as" {
            // `with open(p) as handle`: the value is before `as`, the binding
            // after it.
            let value = *children[..at].iter().rev().find(|&&child| !self.is_layout(child))?;
            let targets: Vec<String> = children.get(at + 1).map(|&child| self.pattern_names(child)).unwrap_or_default();
            return Some((targets, value));
        }
        let targets = children[..at]
            .iter()
            .filter(|&&child| !matches!(self.kind(child), "for" | "(" | ")"))
            .flat_map(|&child| self.pattern_names(child))
            .collect();
        let value = *children.get(at + 1)?;
        Some((targets, value))
    }

    // -- imports ------------------------------------------------------------

    fn import(&mut self, id: u32) {
        let kind = self.kind(id);
        match kind {
            "import_declaration" if self.kinds.family == Family::Go => self.go_import(id),
            "import_declaration" if self.kinds.family == Family::Java => self.java_import(id),
            "import_statement" | "import_from_statement" => {
                if self.is_js() {
                    self.js_import(id);
                } else {
                    self.python_import(id);
                }
            }
            "export_statement" => {
                // `export ... from "x"` re-exports; a plain `export` holds a
                // declaration, which the walk reaches on its own; `export
                // default <expr>` is the expression.
                if self.child_of_kind(id, "string").is_some() {
                    self.js_import(id);
                } else if !self.children(id).into_iter().any(|child| {
                    matches!(
                        self.kind(child),
                        "function_declaration" | "class_declaration" | "lexical_declaration" | "variable_declaration"
                            | "generator_function_declaration" | "type_alias_declaration" | "interface_declaration"
                    )
                }) {
                    self.expression_statement(id);
                }
            }
            _ => {}
        }
    }

    /// `import ("fmt"; str "strings")`: one import per spec, the module as
    /// written and the alias the spec binds — an explicit name, or the last
    /// path segment. Go gives all of them the import clause's line, as the
    /// lexer does, because the clause is the statement.
    fn go_import(&mut self, id: u32) {
        let line = self.line(id);
        let mut specs = Vec::new();
        let mut walk = vec![id];
        while let Some(node) = walk.pop() {
            for child in self.children(node) {
                if self.kind(child) == "import_spec" {
                    specs.push(child);
                } else if matches!(self.kind(child), "import_spec_list") {
                    walk.push(child);
                }
            }
        }
        for spec in specs {
            let children = self.children(spec);
            let Some(string) = children
                .iter()
                .copied()
                .find(|&child| matches!(self.kind(child), "interpreted_string_literal" | "raw_string_literal"))
            else {
                continue;
            };
            let module = self.string_value(string);
            let alias = children
                .iter()
                .copied()
                .find(|&child| self.kind(child) == "identifier")
                .map(|child| self.text_string(child))
                .unwrap_or_else(|| module.rsplit('/').next().unwrap_or(&module).to_string());
            self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
        }
    }

    /// `import java.util.List;` binds the last segment; `import java.io.*;`
    /// binds the package, which is a star import with no local name.
    fn java_import(&mut self, id: u32) {
        let line = self.line(id);
        let mut parts = Vec::new();
        let mut star = false;
        for child in self.children(id) {
            if self.kind(child) == "*" {
                star = true;
                continue;
            }
            // The segments are read by their text, not their kind: `when`,
            // `module` and `record` are contextual keywords this grammar gives
            // their own kind, and an import of `Mockito.when` names one.
            if let Some(name) = self.leaf_name(child) {
                if name == "import" || (name == "static" && parts.is_empty()) {
                    continue;
                }
                parts.push(name);
            }
        }
        let module = parts.join(".");
        if module.is_empty() {
            return;
        }
        if star {
            self.facts.imports.push(Import { line, module, kind: IMPORT_STAR, ..Import::default() });
        } else {
            let alias = parts.last().cloned().unwrap_or_default();
            self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
        }
    }

    fn python_import(&mut self, id: u32) {
        let line = self.line(id);
        let children = self.children(id);
        if self.kind(id) == "import_from_statement" {
            // `from <module> import <bindings>`. The module may be relative
            // (`.`, `..pkg.mod`) and the bindings may sit in a parenthesised
            // `import_list`, which is one list of names, not a wrapper to skip.
            let module_at = children
                .iter()
                .position(|&child| matches!(self.kind(child), "dotted_name" | "identifier" | "relative_import"));
            let module = module_at.map(|at| self.text_string(children[at])).unwrap_or_default();
            let bindings = module_at.map(|at| &children[at + 1..]).unwrap_or(&children[..]);
            self.import_bindings(bindings, &module, line);
            return;
        }
        // `import a.b as c, d`: each clause is a module and an optional alias.
        // The alias is a name of its own, not the module of the next clause.
        let mut index = 0;
        while index < children.len() {
            let child = children[index];
            let aliased = index > 0 && self.kind(children[index - 1]) == "as";
            if !aliased && matches!(self.kind(child), "dotted_name" | "identifier") {
                let module = self.text_string(child);
                let alias = children
                    .get(index + 2)
                    .filter(|_| children.get(index + 1).is_some_and(|&n| self.kind(n) == "as"))
                    .map(|&n| self.text_string(n))
                    .unwrap_or_else(|| module.clone());
                self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
            }
            index += 1;
        }
    }

    /// The bindings of a `from ... import` clause, in order. `from m import
    /// (a, b as c)`, `from m import *` and `from m import a, b` all land here.
    fn import_bindings(&mut self, children: &[u32], module: &str, line: u32) {
        let mut index = 0;
        while index < children.len() {
            let child = children[index];
            match self.kind(child) {
                "import_list" => {
                    let inner = self.children(child);
                    self.import_bindings(&inner, &module, line);
                }
                "wildcard_import" | "*" => {
                    self.facts.imports.push(Import { line, module: module.into(), kind: IMPORT_STAR, ..Import::default() });
                }
                "aliased_import" => {
                    let inner = self.children(child);
                    let Some(&name_node) = inner.first() else {
                        index += 1;
                        continue;
                    };
                    let name = self.text_string(name_node);
                    let alias = inner
                        .last()
                        .filter(|&&last| last != name_node)
                        .map(|&last| self.text_string(last))
                        .unwrap_or_else(|| name.clone());
                    self.facts.imports.push(Import { line, module: module.into(), name, alias, kind: IMPORT_MODULE });
                }
                "dotted_name" | "identifier" => {
                    let name = self.text_string(child);
                    let aliased = children.get(index + 1).is_some_and(|&next| self.kind(next) == "as");
                    let alias = if aliased {
                        children.get(index + 2).map(|&next| self.text_string(next)).unwrap_or_else(|| name.clone())
                    } else {
                        name.clone()
                    };
                    self.facts.imports.push(Import { line, module: module.into(), name, alias, kind: IMPORT_MODULE });
                    if aliased {
                        index += 2;
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }

    fn js_import(&mut self, id: u32) {
        let line = self.line(id);
        let children = self.children(id);
        let module = children
            .iter()
            .find(|&&child| self.kind(child) == "string")
            .map(|&child| self.string_value(child))
            .unwrap_or_default();
        if module.is_empty() {
            return;
        }
        for child in children {
            match self.kind(child) {
                "import_clause" | "export_clause" => self.js_bindings(child, &module, line),
                "import_specifier" => self.js_specifier(child, &module, line),
                "string" | "import" | "export" | "from" | ";" => {}
                _ => self.js_bindings(child, &module, line),
            }
        }
    }

    fn js_bindings(&mut self, id: u32, module: &str, line: u32) {
        for child in self.children(id) {
            match self.kind(child) {
                "identifier" | "default" | "type" => {
                    let alias = self.text_string(child);
                    if alias != "type" {
                        self.facts.imports.push(Import { line, module: module.into(), alias, kind: IMPORT_MODULE, ..Import::default() });
                    }
                }
                "named_imports" | "named_exports" => {
                    for specifier in self.children(child) {
                        if self.kind(specifier) == "import_specifier" {
                            self.js_specifier(specifier, module, line);
                        }
                    }
                }
                "import_specifier" => self.js_specifier(child, module, line),
                "namespace_import" => {
                    let alias = self.children(child).into_iter().rev().find_map(|node| {
                        (self.kind(node) == "identifier").then(|| self.text_string(node))
                    });
                    self.facts.imports.push(Import {
                        line,
                        module: module.into(),
                        alias: alias.unwrap_or_default(),
                        kind: IMPORT_MODULE,
                        ..Import::default()
                    });
                }
                _ => {}
            }
        }
    }

    fn js_specifier(&mut self, id: u32, module: &str, line: u32) {
        let children = self.children(id);
        let Some(&first) = children.first() else { return };
        let name = self.text_string(first);
        let alias = children
            .iter()
            .position(|&child| self.kind(child) == "as")
            .and_then(|at| children.get(at + 1))
            .map(|&node| self.text_string(node))
            .unwrap_or_else(|| name.clone());
        self.facts.imports.push(Import { line, module: module.into(), name, alias, kind: IMPORT_MODULE });
    }

    fn string_value(&self, id: u32) -> String {
        let text = String::from_utf8_lossy(self.text(id));
        text.trim_matches(|ch| matches!(ch, '"' | '\'' | '`')).to_string()
    }

    // -- values -------------------------------------------------------------

    /// The flow form of one expression, recording every call it contains, in
    /// the same shapes `crate::expr` documents:
    ///
    /// ```text
    /// 0                    a constant
    /// ["n","a.b"]          a name or attribute path
    /// ["m",[e,...]]        values combined (operator, format string, mapping)
    /// ["t",[e,...]]        a tuple or list, element by element
    /// ["c",id,recv,[arg]]  a call; `id` indexes the symbol's call table
    /// ["k","name",e]       a keyword argument, only inside a call's arguments
    /// ```
    fn form(&mut self, id: u32) -> String {
        let kind = self.kind(id);
        if self.is_rust() {
            if let Some(out) = self.rust_form(id) {
                return out;
            }
        }
        if self.kinds.calls.contains(&kind) {
            return self.call_form(id);
        }
        // Python has no boolean keyword in this grammar, but the lexer reads
        // `True`, `False` and `None` as names, so they are names here too.
        if matches!(kind, "True" | "False" | "None") {
            let text = self.text_string(id);
            let mut out = "[\"n\",".to_string();
            push_string(&mut out, &text);
            out.push(']');
            return out;
        }
        if let Some(name) = self.soft_name(id) {
            let mut out = "[\"n\",".to_string();
            push_string(&mut out, &name);
            out.push(']');
            return out;
        }
        match kind {
            "identifier" | "this" | "super" | "self" | "shorthand_property_identifier" => {
                let text = self.text_string(id);
                let text = if text == "this" { "self".to_string() } else { text };
                if text.is_empty() {
                    "0".to_string()
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &text);
                    out.push(']');
                    out
                }
            }
            "attribute" | "member_expression" | "subscript" | "subscript_expression" => {
                let path = self.form_path(id);
                if path.is_empty() {
                    // `f(x)[0].y`: the base is a call, so there is no path, and
                    // the value is the base's — a subscript of a value is that
                    // value. Writing 0 here lost the call's argument.
                    match self.children(id).first().copied() {
                        Some(base) => self.form(base),
                        None => "0".to_string(),
                    }
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &path);
                    out.push(']');
                    out
                }
            }
            "argument_list" | "arguments" | "parenthesized_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.tuple_form(&items),
                }
            }
            "binary_operator" | "comparison_operator" | "boolean_operator" | "binary_expression" | "ternary_expression"
            // `a if c else b` holds three values and every one is read; the
            // condition is where `x if p(v) else y` calls `p`. `(n := f())` is
            // the same: the value is on the right, the name on the left.
            | "conditional_expression" | "named_expression"
            // `xs[lo():hi():st()]`: a slice and a subscript list are lists of
            // values, not wrappers, so nothing may be dropped.
            | "slice" | "subscript_list" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                if items.len() == 1 {
                    return self.form(items[0]);
                }
                self.merge_form(&items)
            }
            "not_operator" | "unary_operator" | "unary_expression" | "await_expression" | "yield_expression"
            | "typeof" | "void" | "non_null_expression" | "as_expression" | "type_assertion" | "delete_expression" => {
                match self.expression_child(id) {
                    Some(child) => {
                        let inner = self.form(child);
                        format!("[\"m\",[{inner}]]")
                    }
                    None => "0".to_string(),
                }
            }
            // `a if p(x) else b`: the condition is an expression of its own and
            // the call in it is a fact. The lexer's flow form reads the whole
            // conditional, so the CST must too.
            "conditional_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            // `xs[1:min(len(xs), n)]`, `for x in a, b`: a slice or an
            // expression list is a run of expressions and every one of them
            // can carry a call. Reading only the first would drop the bound.
            "slice" | "open_slice" | "subscript_list" | "expression_list" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.tuple_form(&items),
                }
            }
            // `z: "Thing"` with no value: the statement is the annotation, and
            // the name in it is what the lexer reads as the expression. With a
            // value (`z: "Thing" = f()`) the value is what the statement is.
            "annotated" => {
                let children = self.children(id);
                let has_value = children.iter().any(|&child| self.kind(child) == "=");
                match (has_value, children.last()) {
                    (true, Some(&last)) => self.form(last),
                    (false, Some(&first)) => self.form(first),
                    _ => "0".to_string(),
                }
            }
            // `path := f(x)`: a walrus expression is the value, and the name
            // is a binding.
            "named_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => {
                        let value = items[items.len() - 1];
                        self.form(value)
                    }
                }
            }
            // `lambda x: f(x)`: the body is the expression the lambda yields.
            "lambda" => match self.children(id).into_iter().rev().find(|&child| !self.is_layout(child)) {
                Some(body) => self.form(body),
                None => "0".to_string(),
            },
            "tuple" | "list" | "set" | "pattern_list" | "array" | "tuple_pattern" | "list_pattern" | "array_pattern" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                if items.is_empty() {
                    "0".to_string()
                } else {
                    self.tuple_form(&items)
                }
            }
            "list_comprehension" | "set_comprehension" | "generator_expression" | "dictionary_comprehension" => {
                // `[f(x) for x in xs if p(x)]`: the element and every clause's
                // value are what the reader needs, and the clauses carry calls.
                let mut items: Vec<u32> = Vec::new();
                for child in self.children(id) {
                    if self.is_layout(child) {
                        continue;
                    }
                    items.push(child);
                }
                if items.is_empty() {
                    "0".to_string()
                } else {
                    self.tuple_form(&items)
                }
            }
            "for_in_clause" | "if_clause" | "comprehension_clause" => {
                // `for x in xs` / `if p(x)`: the value is the last child; the
                // target is a binding, not a value.
                match self.children(id).into_iter().rev().find(|&child| !self.is_layout(child)) {
                    Some(child) => self.form(child),
                    None => "0".to_string(),
                }
            }
            "dictionary" | "object" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                if items.is_empty() {
                    "0".to_string()
                } else {
                    self.merge_form(&items)
                }
            }
            "pair" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => {
                        let key = self.form(items[0]);
                        let value = self.form(items[items.len() - 1]);
                        format!("[\"m\",[{key},{value}]]")
                    }
                }
            }
            "keyword_argument" => {
                // `f(default=True)` is `name`, `=`, `value`: the separator is a
                // child, and taking the children as a pair read the value as
                // the `=` and every keyword argument as a constant.
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && self.kind(child) != "=")
                    .collect();
                // The grammar writes the `=` sometimes as a child and sometimes
                // not, so the name is the first item and the value the last.
                match (items.first().copied(), items.last().copied()) {
                    (Some(name), Some(value)) if items.len() >= 2 => {
                        let mut out = "[\"k\",".to_string();
                        push_string(&mut out, &self.text_string(name));
                        out.push(',');
                        out.push_str(&self.form(value));
                        out.push(']');
                        out
                    }
                    _ => "0".to_string(),
                }
            }
            "spread_element" | "rest_pattern" | "list_splat" | "list_splat_pattern" => match self.expression_child(id) {
                Some(child) => self.form(child),
                None => "0".to_string(),
            },
            "assignment" | "assignment_expression" | "augmented_assignment_expression" => {
                let children = self.children(id);
                children.last().filter(|&&last| last != children[0]).map(|&last| self.form(last)).unwrap_or_else(|| "0".to_string())
            }
            // `lambda x: f(x)`: the body is an expression of its own and it is
            // where the calls are; the parameters are names, not values.
            "lambda" => match self.children(id).into_iter().rev().find(|&child| !self.is_layout(child)) {
                Some(body) => self.form(body),
                None => "0".to_string(),
            },
            // `f"{a(x)} b{c(y)}"` and `` `a${h(x)}b${g(y)}` ``: the text is not
            // a value, but each field's expression is, and the one-pass lexer's
            // string reader forms the same merge of them. So a call written in
            // a field is a fact on both paths, and a field with no call saves
            // nothing to compare.
            "f_string" | "template_string" => {
                // Python's replacement field is `interpolation`, JavaScript's
                // is `template_substitution`; everything else is the same
                // shape, and a template with no substitution is still `0`.
                let field = if kind == "f_string" { "interpolation" } else { "template_substitution" };
                let mut parts = Vec::new();
                for child in self.children(id) {
                    if self.kind(child) != field {
                        continue;
                    }
                    // `{x!r:>{w}}` holds one expression; the conversion and the
                    // format spec are text, and their nested fields are
                    // `interpolation` nodes of their own, so they are reached
                    // from the spec, not from here. A template substitution's
                    // nested substitutions live inside a nested `template_string`
                    // node, which `form` reaches as the substitution's value.
                    let value = self.children(child).into_iter().find(|&part| {
                        !self.is_layout(part) && self.node(part).first_child != NONE && self.kind(part) != field
                    });
                    if let Some(value) = value {
                        parts.push(self.form(value));
                    }
                }
                match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.remove(0),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                }
            }
            // `UnsafeCell<State>`, `Vec<Entry>`: a generic type holds the base
            // and the type arguments, and both are names the lexer reads.
            "generic_type" | "type_arguments" | "type_parameter_list" => {
                let mut parts = Vec::new();
                for child in self.children(id) {
                    if self.is_layout(child) || matches!(self.kind(child), "<" | ">" | "," | "::") {
                        continue;
                    }
                    let value = self.form(child);
                    if value != "0" {
                        parts.push(value);
                    }
                }
                match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.remove(0),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                }
            }
            // `[u8; CLASSES]`: an array type holds a type name and a length, both
            // of them names the lexer reads.
            "array_type" | "slice_type" | "reference_type" | "pointer_type" => {
                let mut parts = Vec::new();
                for child in self.children(id) {
                    if self.is_layout(child) || matches!(self.kind(child), "[" | "]" | "&" | "*" | ";" ) {
                        continue;
                    }
                    let value = self.form(child);
                    if value != "0" {
                        parts.push(value);
                    }
                }
                match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.remove(0),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                }
            }
            "string" | "string_literal" | "number" | "integer" | "float" | "true" | "false"
            | "none" | "null" | "undefined" | "regex" | "comment" => "0".to_string(),
            // A region the grammar could not name. The tokens are still there —
            // recovery keeps every byte — so the values inside it are read like
            // any other: `("prefix " f"{call(x)}")` is an ERROR today because
            // implicit concatenation of a literal with an f-string is not a
            // rule, and skipping the region dropped the call inside it.
            "ERROR" => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && self.node(child).first_child != NONE)
                    .collect();
                // The tokens inside a region the grammar could not name are
                // still there, and a call is visible in them: a name followed by
                // an argument list. Reading that is not inventing a fact — it is
                // reading what recovery left — and it is the only way a call
                // inside `("prefix " f"{call(x)}")` survives, since implicit
                // concatenation of a literal with an f-string is not a rule yet.
                self.recovered_calls(id);
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            // A keyword reused as an identifier (`match`, `type`, `case`)
            // carries that keyword as its kind; the spelling is the name.
            _ if self.leaf_name(id).is_some() => {
                let text = self.leaf_name(id).unwrap_or_default();
                let text = if text == "this" { "self".to_string() } else { text };
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &text);
                out.push(']');
                out
            }
            _ => {
                // A node with no mapping: its first value-bearing child, so a
                // wrapper the grammar added costs nothing.
                match self.expression_child(id) {
                    Some(child) if self.node(child).first_child != NONE => self.form(child),
                    _ => "0".to_string(),
                }
            }
        }
    }


    /// Calls visible in the raw tokens of an `ERROR` region: `name(` or
    /// `a.b(`. Nothing is invented — the name and the argument list are both in
    /// the source, and the region is only reached where the grammar failed to
    /// build a node for them.
    fn recovered_calls(&mut self, id: u32) {
        let tokens: Vec<u32> = self.children(id);
        let mut at = 0;
        while at < tokens.len() {
            if self.kind(tokens[at]) != "identifier" {
                at += 1;
                continue;
            }
            // `a.b(` walks the dotted chain forward before the paren, and the
            // name keeps every component: the resolver matches on the path.
            let mut end = at;
            let mut name = self.text_string(tokens[at]);
            while end + 2 < tokens.len()
                && matches!(self.kind(tokens[end + 1]), "." | "?.")
                && self.kind(tokens[end + 2]) == "identifier"
            {
                name.push('.');
                name.push_str(&self.text_string(tokens[end + 2]));
                end += 2;
            }
            if end + 1 < tokens.len() && self.kind(tokens[end + 1]) == "(" {
                let line = self.line(tokens[at]);
                self.push_call(name, line);
            }
            at = end + 1;
        }
    }

    fn tuple_form(&mut self, items: &[u32]) -> String {
        let mut out = "[\"t\",[".to_string();
        for (index, &item) in items.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&self.form(item));
        }
        out.push_str("]]");
        out
    }

    fn merge_form(&mut self, items: &[u32]) -> String {
        let mut out = "[\"m\",[".to_string();
        for (index, &item) in items.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&self.form(item));
        }
        out.push_str("]]");
        out
    }

    fn call_form(&mut self, id: u32) -> String {
        self.call_form_link(id).0
    }

    /// A call's form, and — when the callee is a name — the index it was
    /// recorded under together with the receiver it is called on, so the call
    /// above it can name its receiver without guessing: `A().run` links `.run`
    /// to `(0, "0")`, the call `A`.
    fn call_form_link(&mut self, id: u32) -> (String, Option<(u32, String)>) {
        let children = self.children(id);
        let is_new = self.kind(id) == "new_expression";
        let callee = children
            .iter()
            .copied()
            .find(|&child| !matches!(self.kind(child), "new" | "argument_list" | "arguments" | "type_arguments" | "(" | ")"))
            .unwrap_or(id);
        // `type("H", (), {...})()`: the callee is a value produced by another
        // call. That inner call is the fact and its arguments are where the
        // dictionary (and everything in it) lives; the outer call has no name
        // of its own, so it adds none. `callee != id` keeps a recovered call
        // whose own callee is itself from descending forever.
        if callee != id && matches!(self.kind(callee), "call" | "call_expression" | "new_expression") {
            let inner = self.form(callee);
            if let Some(list) = children.iter().copied().find(|&child| matches!(self.kind(child), "argument_list" | "arguments")) {
                self.arguments(list);
            }
            return (inner, None);
        }
        // `f().g()`: the receiver is a call, and it is a fact of its own. The
        // call's own index and receiver go in the slot above it, so the record
        // says which call produced the receiver: `A().run` records `A`, `.run`
        // and the link between them. The calls array alone cannot say it — in
        // `TaintAnalyzer(a, b(), c).run()` the argument `b()` is recorded
        // between the two — and the path keeps the name the source spells.
        let mut receiver_call = None;
        if matches!(self.kind(callee), "attribute" | "member_expression") {
            if let Some(&object) = self.children(callee).first() {
                if self.named_call(object) {
                    // `new URL(x).toString()`, `A().b()`, `A().b().c()`: a
                    // receiver that is a call the walk records, whose own
                    // receiver is carried along so a chain stays readable one
                    // call at a time.
                    // The receiver is the inner call's whole form, arguments
                    // included: rebuilding it from the link alone wrote `[]` for
                    // every argument and lost `Path(path)` in `Path(path).resolve()`.
                    let (form, _link) = self.call_form_link(object);
                    receiver_call = Some(form);
                } else {
                    // `[...xs].filter(f)`, `handlers[k](x)`: any expression,
                    // whose calls are facts of their own. Its value is also the
                    // receiver the lexer writes: `(a or b).split('-')` has
                    // `a or b` there, and dropping it lost every name the
                    // receiver held.
                    let value = self.form(object);
                    if self.path_of(object).is_empty() {
                        receiver_call = Some(value);
                    }
                }
            }
        } else if !matches!(self.kind(callee), "identifier" | "property_identifier" | "type_identifier") {
            // `(_SockFilter * len(filters))(*filters)`, `handlers[k](x)` and
            // `f()()`: a callee that is not a plain name is an expression, and
            // the calls inside it are facts. A name has none, so it is skipped.
            self.form(callee);
        }
        let (path, receiver) = self.call_target(callee, receiver_call);
        let line = self.call_line(callee, id);
        let index = self.push_call(path, line);
        let args = children
            .iter()
            .copied()
            .find(|&child| matches!(self.kind(child), "argument_list" | "arguments"));
        let mut out = format!("[\"c\",{index},{receiver},");
        match args {
            Some(list) => out.push_str(&self.arguments(list)),
            None => out.push_str("[]"),
        }
        out.push(']');
        let _ = is_new;
        let link = (index as u32, receiver);
        (out, Some(link))
    }

    /// The called path and the receiver form: `f(x)` -> (`f`, `0`),
    /// `a.b.c(x)` -> (`a.b.c`, `["n","a.b"]`), `A().b()` -> (`.b`,
    /// `["c",0,0,[]]`) with the index of the `A` call the walk recorded.
    fn call_target(&self, callee: u32, receiver_call: Option<String>) -> (String, String) {
        match self.kind(callee) {
            "attribute" | "member_expression" => {
                let children = self.children(callee);
                let name = self.member_name(callee);
                let object = children.first().copied();
                let receiver_path = object.map(|node| self.path_of(node)).unwrap_or_default();
                let path = if receiver_path.is_empty() {
                    format!(".{name}")
                } else {
                    format!("{receiver_path}.{name}")
                };
                let receiver = if !receiver_path.is_empty() {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &receiver_path);
                    out.push(']');
                    out
                } else {
                    receiver_call.unwrap_or_else(|| "0".to_string())
                };
                (path, receiver)
            }
            "call" | "call_expression" | "new_expression" => {
                // `f()(...)`: the callee is an inner call, and the name is the
                // inner call's own callee. It has to be looked for *inside*
                // `callee`: a call node reaches this arm as its own callee when
                // the grammar recovers a call with no operand, and recursing on
                // the same index is an infinite descent that takes the process
                // stack out (a four kilobyte fuzz file is enough).
                let inner = self.children(callee).into_iter().find(|&child| {
                    !matches!(self.kind(child), "(" | ")" | "argument_list" | "arguments" | "type_arguments" | "new")
                });
                match inner {
                    Some(inner) => {
                        let (path, _) = self.call_target(inner, None);
                        (path, "0".to_string())
                    }
                    None => (String::new(), "0".to_string()),
                }
            }
            _ => {
                let path = self.path_of(callee);
                (path, "0".to_string())
            }
        }
    }

    /// The property name of an `a.b` node: the last leaf after the dot. It is
    /// read by position, not by kind, because a member called `get` carries the
    /// grammar's `get` kind and a private one carries `#`.
    fn member_name(&self, id: u32) -> String {
        for &child in self.children(id).iter().rev() {
            if self.node(child).first_child != NONE {
                continue;
            }
            let text = self.text(child);
            if text.is_empty() {
                continue;
            }
            if text[0].is_ascii_alphanumeric() || text[0] == b'_' || text[0] == b'#' || text[0] == b'$' {
                return String::from_utf8_lossy(text).trim_start_matches('#').to_string();
            }
        }
        String::new()
    }

    /// A dotted path, recording the calls on the way: `urlparse(x).netloc` has
    /// a call under the attribute, and that call is a fact.
    fn form_path(&mut self, id: u32) -> String {
        let kind = self.kind(id);
        let children = self.children(id);
        if matches!(kind, "attribute" | "member_expression" | "subscript" | "subscript_expression") {
            if let Some(&object) = children.first() {
                // A receiver can be any expression; nothing is recorded for a
                // plain name, so this costs a walk and never a wrong fact.
                self.form(object);
            }
        }
        // `name[len(prefix):]`: the index is an expression of its own.
        if matches!(kind, "subscript" | "subscript_expression") {
            for &child in children.iter().skip(1) {
                if !self.is_layout(child) {
                    self.form(child);
                }
            }
        }
        self.path_of(id)
    }

    /// The line a call is written on. For `path\n  .replace(...)` that is the
    /// line of `replace`, which is what a reader points at; Tree-sitter's node
    /// start would say the line of `path`.
    fn call_line(&self, callee: u32, call: u32) -> u32 {
        match self.kind(callee) {
            "attribute" | "member_expression" => {
                let children = self.children(callee);
                if let Some(&name) = children.iter().rev().find(|&&child| {
                    self.node(child).first_child == NONE && !self.is_layout(child)
                }) {
                    return self.line(name);
                }
                self.line(call)
            }
            _ => self.line(callee),
        }
    }

    fn arguments(&mut self, list: u32) -> String {
        let mut out = "[".to_string();
        let mut first = true;
        for child in self.children(list) {
            if self.is_layout(child) {
                continue;
            }
            if !first {
                out.push(',');
            }
            first = false;
            // Go and Java do not use `form`: an argument that is itself a call
            // (`path.Join(os.Getenv(..))`) would come back as `0`.
            let value = match self.kinds.family {
                Family::Go | Family::Java => self.decl_value(child),
                _ => self.form(child),
            };
            out.push_str(&value);
        }
        out.push(']');
        out
    }

    /// A dotted name off the source: `a . b` and `a.b` are the same path, and
    /// `this` becomes `self` as the flow form spells it.
    fn path_of(&self, id: u32) -> String {
        if let Some(name) = self.soft_name(id) {
            return name;
        }
        if self.is_rust() {
            if let Some(path) = self.rust_path(id) {
                return path;
            }
        }
        match self.kind(id) {
            "identifier" | "property_identifier" | "private_property_identifier" | "type_identifier"
            | "shorthand_property_identifier" | "this" | "super" => {
                let text = self.text_string(id);
                if text == "this" {
                    "self".to_string()
                } else {
                    text.trim_start_matches('#').to_string()
                }
            }
            "attribute" | "member_expression" => {
                let children = self.children(id);
                let name = self.member_name(id);
                let base = children.first().map(|&child| self.path_of(child)).unwrap_or_default();
                if base.is_empty() {
                    format!(".{name}")
                } else {
                    format!("{base}.{name}")
                }
            }
            "subscript" | "subscript_expression" => {
                let base = self.children(id).first().map(|&child| self.path_of(child)).unwrap_or_default();
                base
            }
            "parenthesized_expression" => self.expression_child(id).map(|child| self.path_of(child)).unwrap_or_default(),
            "as_expression" | "type_assertion" | "non_null_expression" => {
                self.expression_child(id).map(|child| self.path_of(child)).unwrap_or_default()
            }
            // A call has no name of its own: `f().x` names `.x`.
            "call" | "call_expression" | "new_expression" => String::new(),
            // `match.group`, `type.name`: a keyword reused as an identifier.
            _ => self.leaf_name(id).map(|text| if text == "this" { "self".to_string() } else { text }).unwrap_or_default(),
        }
    }

    /// The path of a value used as an assignment target, or empty when the
    /// target is not a name (a literal index, a call).
    fn value_path(&self, id: u32) -> Option<String> {
        if let Some(name) = self.soft_name(id) {
            return Some(name);
        }
        match self.kind(id) {
            "identifier" | "attribute" | "member_expression" | "subscript" | "subscript_expression" | "this" => {
                let path = self.path_of(id);
                (!path.is_empty()).then_some(path)
            }
            "tuple_pattern" | "pattern_list" | "array_pattern" | "list_pattern" | "object_pattern" => None,
            _ => self.leaf_name(id).filter(|path| !path.is_empty()),
        }
    }

    // -- Rust ---------------------------------------------------------------
    //
    // Rust is read here rather than through the Python/JS arms above: its
    // declarations are items, its calls are `call_expression` with a
    // `field_expression` or `scoped_identifier` callee, and a `use` is a path,
    // not a binding list. Every function below is reached only when
    // `Kinds::rust` is set, so nothing here can change another language.

    /// The Rust-only nodes, in the order they are met. `false` means "not mine,
    /// let the shared walk read it" (an `expression_statement`, a block, a
    /// plain value).

    /// `const NAME: Type = value;`: the target is the name, the value is what
    /// follows the `=`.
    fn rust_const(&mut self, id: u32) {
        let children = self.children(id);
        let name = children
            .iter()
            .copied()
            .find(|&child| matches!(self.kind(child), "identifier" | "default"))
            .map(|child| self.text_string(child))
            .unwrap_or_default();
        if name.is_empty() || name == "_" {
            return;
        }
        let Some(equal) = children.iter().position(|&child| self.kind(child) == "=") else {
            return;
        };
        let Some(&value) = children.get(equal + 1) else { return };
        let value = self.form(value);
        let line = self.line(id);
        // The target list is a plain array of names, as the lexer writes it.
        let mut targets = "[\"".to_string();
        targets.push_str(&name);
        targets.push_str("\"]");
        self.event(&format!("[\"a\",{line},{targets},{value},0]"));
    }


    /// `{ free: [u8; N], bump: u8 }`: one event for the field list, the way the
    /// lexer reads the body.
    fn rust_field_list(&mut self, id: u32) {
        let mut parts = Vec::new();
        for child in self.children(id) {
            if self.is_layout(child) || !matches!(self.kind(child), "field_declaration" | "field_declaration_list") {
                continue;
            }
            // The field's own children are the name and the type: read them
            // one by one, the way the lexer reads the field as an expression.
            let mut field = Vec::new();
            for part in self.children(child) {
                if self.is_layout(part) {
                    continue;
                }
                let value = self.form(part);
                if value != "0" {
                    field.push(value);
                }
            }
            if !field.is_empty() {
                parts.push(format!("[\"m\",[{}]]", field.join(",")));
            }
        }
        if parts.is_empty() {
            return;
        }
        // The lexer stamps the statement on the first field's line.
        let line = self
            .children(id)
            .into_iter()
            .find(|&child| self.kind(child) == "field_declaration")
            .map(|child| self.line(child))
            .unwrap_or_else(|| self.line(id));
        self.event(&format!("[\"x\",{line},[\"t\",[{}]]]", parts.join(",")));
    }

    fn enter_rust(&mut self, id: u32) -> bool {
        match self.kind(id) {
            "impl_item" => {
                self.rust_impl(id);
                true
            }
            "use_declaration" => {
                self.rust_import(id);
                true
            }
            "let_declaration" => {
                self.rust_let(id);
                true
            }
            // `const X: usize = 4;` and `static Y: &str = "s";`: the lexer
            // reads a module-level binding and files an assignment event; the
            // item was never read here, so every module constant was lost.
            "const_item" | "static_item" => {
                self.rust_const(id);
                true
            }

            // A struct's fields are declarations, but the one-pass lexer reads
            // the body as an expression statement and files one event for it.
            // The names are real either way, and without an event the file
            // looks like the CST lost them.
            "field_declaration_list" => {
                self.rust_field_list(id);
                true
            }
            "return_expression" => {
                let line = self.line(id);
                let value = match self.expression_child(id) {
                    Some(child) => self.form(child),
                    None => "0".to_string(),
                };
                self.event(&format!("[\"r\",{line},{value}]"));
                true
            }
            "if_expression" | "while_expression" => {
                self.rust_condition(id);
                self.event("[\"b\"]");
                true
            }
            "loop_expression" => {
                self.event("[\"b\"]");
                true
            }
            "for_expression" => {
                self.rust_for(id);
                true
            }
            "match_expression" => {
                // `match c.peek() {`: the scrutinee is a value the lexer reads
                // as an expression statement before the block opens.
                self.rust_condition(id);
                self.event("[\"b\"]");
                true
            }
            "match_arm" => {
                // `pat => expr,`: the arm's value is an expression of its own,
                // and where the calls written after `=>` live.
                if let Some(value) = self.rust_after_arrow(id) {
                    self.form(value);
                }
                self.event("[\"e\"]");
                false
            }
            _ => false,
        }
    }

    /// The control-flow markers a Rust expression closes with. `if`/`while`/
    /// `loop`/`for`/`match` are expressions here, so they are not the shared
    /// `_statement` kinds the other arms close.
    fn leave_rust(&mut self, id: u32) {
        if matches!(self.kind(id), "if_expression" | "while_expression" | "loop_expression" | "for_expression" | "match_expression") {
            self.event("[\"j\"]");
        }
    }

    /// `impl Reader { ... }`, `impl Draw for Widget { ... }`: a container named
    /// after the type it implements, so its methods take that type as owner and
    /// their calls sit on them. A trait impl records the trait as a base.
    fn rust_impl(&mut self, id: u32) {
        let line = self.line(id);
        let mut inherent = String::new();
        let mut trait_name = String::new();
        let mut seen_for = false;
        for child in self.children(id) {
            match self.kind(child) {
                "impl" | "type_parameters" | "where_clause" | "declaration_list" => {}
                "for" => seen_for = true,
                _ => {
                    let name = self.rust_type_name(child);
                    if name.is_empty() {
                        continue;
                    }
                    if seen_for && inherent.is_empty() {
                        inherent = name;
                    } else if !seen_for && trait_name.is_empty() {
                        trait_name = name;
                    }
                }
            }
        }
        let (name, base) = if inherent.is_empty() { (trait_name, String::new()) } else { (inherent, trait_name) };
        if name.is_empty() {
            return;
        }
        let index = self.add_symbol(name.clone(), String::new(), KIND_CLASS, line, Vec::new());
        if !base.is_empty() {
            self.facts.symbols[index].bases.push(base);
        }
        let symbol = self.frame().symbol;
        self.stack.push(Frame { node: id, symbol, class: name, receiver: String::new(), container: true, defines: Some(index) });
    }

    /// The spelled name of a type node: `Reader`, `Reader<'a>`, `fmt::Display`.
    fn rust_type_name(&self, id: u32) -> String {
        match self.kind(id) {
            "type_identifier" | "identifier" => self.text_string(id),
            "generic_type" | "reference_type" | "scoped_type_identifier" | "tuple_type" => {
                self.last_name(id).unwrap_or_default()
            }
            _ => String::new(),
        }
    }

    /// `let mut value = f(x);` / `let x: T;`. A binding is an assignment event
    /// when it carries a value and a name event when it does not, as the lexer
    /// writes it. The pattern is the child after `let`; the value is the child
    /// after `=`.
    fn rust_let(&mut self, id: u32) {
        let children = self.children(id);
        let Some(&pattern) = children.iter().find(|&&child| !matches!(self.kind(child), "let" | ";" | ":")) else {
            return;
        };
        let line = self.line(id);
        let targets = self.rust_pattern_names(pattern);
        match children.iter().position(|&child| self.kind(child) == "=") {
            Some(equal) if children.len() > equal + 1 => {
                let value = children[equal + 1];
                self.assignment_parts(targets, value, line, false);
            }
            _ => {
                if targets.len() == 1 {
                    let mut text = format!("[\"x\",{line},[\"n\",");
                    push_string(&mut text, &targets[0]);
                    text.push_str("]]");
                    self.event(&text);
                }
            }
        }
    }

    /// The value side of a `match` arm: the child after `=>`.
    fn rust_after_arrow(&self, id: u32) -> Option<u32> {
        let children = self.children(id);
        let at = children.iter().position(|&child| self.kind(child) == "=>")?;
        children.get(at + 1).copied()
    }

    /// The value side of a `let_condition` (`if let Ok(x) = f()`): the child
    /// after `=`. The pattern is a binding, not a value.
    fn rust_let_value(&self, id: u32) -> Option<u32> {
        let children = self.children(id);
        let at = children.iter().position(|&child| self.kind(child) == "=")?;
        children.get(at + 1).copied()
    }

    /// The names a Rust pattern binds. A tuple-struct pattern's first child is
    /// the variant (`Ok(name)` binds `name`, not `Ok`) and a struct pattern's
    /// field names are its own, so neither is read as a binder.
    fn rust_pattern_names(&self, id: u32) -> Vec<String> {
        match self.kind(id) {
            "identifier" | "shorthand_field_identifier" | "self" => vec![self.text_string(id)],
            "mut_pattern" | "ref_pattern" | "reference_pattern" => self.first_identifier(id).into_iter().collect(),
            "tuple_struct_pattern" => {
                self.children(id).into_iter().skip(1).flat_map(|child| self.rust_pattern_names(child)).collect()
            }
            "struct_pattern" => self
                .children(id)
                .into_iter()
                .filter(|&child| matches!(self.kind(child), "field_pattern" | "shorthand_field_identifier" | "remaining_field_pattern"))
                .flat_map(|child| self.rust_pattern_names(child))
                .collect(),
            // `Struct { field: binder }` binds `binder`; `Struct { field }`
            // binds the field's own name.
            "field_pattern" => self
                .children(id)
                .into_iter()
                .rev()
                .find_map(|child| self.first_identifier(child))
                .into_iter()
                .collect(),
            "tuple_pattern" | "or_pattern" | "slice_pattern" | "tuple_expression" => {
                self.children(id).into_iter().flat_map(|child| self.rust_pattern_names(child)).collect()
            }
            _ => self.pattern_names(id),
        }
    }

    /// `if cond {`, `while cond {`: the condition is a value of its own and its
    /// calls are facts. `if let pat = value` is read on the value side, which is
    /// where the calls are.
    fn rust_condition(&mut self, id: u32) {
        for child in self.children(id) {
            match self.kind(child) {
                "if" | "while" => continue,
                "block" | "else_clause" => return,
                "let_condition" | "let_chain" => {
                    if let Some(value) = self.rust_let_value(child) {
                        let value = self.form(value);
                        if value != "0" {
                            let line = self.line(child);
                            self.event(&format!("[\"x\",{line},{value}]"));
                        }
                    }
                    return;
                }
                _ => {
                    if self.is_layout(child) {
                        continue;
                    }
                    let value = self.form(child);
                    if value != "0" {
                        let line = self.line(child);
                        self.event(&format!("[\"x\",{line},{value}]"));
                    }
                    return;
                }
            }
        }
    }

    /// `for value in values {`: the loop binds its pattern to the iterable, so
    /// the record gets an assignment event and a block, as the lexer writes.
    fn rust_for(&mut self, id: u32) {
        let children = self.children(id);
        let Some(at) = children.iter().position(|&child| self.kind(child) == "in") else {
            self.event("[\"b\"]");
            return;
        };
        self.event("[\"b\"]");
        let targets = children[..at]
            .iter()
            .filter(|&&child| !matches!(self.kind(child), "for" | "&" | "ref" | "mut"))
            .flat_map(|&child| self.rust_pattern_names(child))
            .collect();
        if let Some(&value) = children.get(at + 1) {
            let line = self.line(id);
            self.assignment_parts(targets, value, line, false);
        }
    }

    /// `use std::path::PathBuf;`, `use x as y;`, `use a::b::*;`,
    /// `use a::{b, c as d};` — the path import the lexer spells with dots.
    fn rust_import(&mut self, id: u32) {
        let line = self.line(id);
        for child in self.children(id) {
            match self.kind(child) {
                "use" | ";" => {}
                "use_as_clause" => {
                    let inner = self.children(child);
                    let Some(&path) = inner.first() else { continue };
                    let module = self.rust_module_text(path);
                    let alias = inner.last().filter(|&&last| last != path).map(|&last| self.text_string(last));
                    self.facts.imports.push(Import {
                        line,
                        module,
                        alias: alias.unwrap_or_default(),
                        kind: IMPORT_MODULE,
                        ..Import::default()
                    });
                }
                "use_wildcard" => {
                    let Some(&path) = self.children(child).first() else { continue };
                    self.facts.imports.push(Import {
                        line,
                        module: self.rust_module_text(path),
                        kind: IMPORT_STAR,
                        ..Import::default()
                    });
                }
                "scoped_use_list" => {
                    let inner = self.children(child);
                    let Some(&path) = inner.first() else { continue };
                    let module = self.rust_module_text(path);
                    let Some(&list) = inner.iter().find(|&&node| self.kind(node) == "use_list") else { continue };
                    self.rust_use_list(list, &module, line);
                }
                "use_list" => self.rust_use_list(child, "", line),
                _ => {
                    let module = self.rust_module_text(child);
                    if module.is_empty() {
                        continue;
                    }
                    let alias = module.rsplit('.').next().unwrap_or("").to_string();
                    self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
                }
            }
        }
    }

    fn rust_use_list(&mut self, id: u32, module: &str, line: u32) {
        for child in self.children(id) {
            match self.kind(child) {
                "use_as_clause" => {
                    let inner = self.children(child);
                    let Some(&path) = inner.first() else { continue };
                    let name = self.rust_module_text(path);
                    let alias = inner.last().filter(|&&last| last != path).map(|&last| self.text_string(last));
                    self.facts.imports.push(Import {
                        line,
                        module: module.into(),
                        name,
                        alias: alias.unwrap_or_default(),
                        kind: IMPORT_MODULE,
                    });
                }
                "identifier" | "scoped_identifier" | "type_identifier" | "self" => {
                    let name = self.rust_module_text(child);
                    let (module_name, member) = match module.rsplit_once('.') {
                        _ if module.is_empty() => (String::new(), name.clone()),
                        _ => (module.to_string(), name.clone()),
                    };
                    self.facts.imports.push(Import {
                        line,
                        module: module_name,
                        name: member.clone(),
                        alias: member,
                        kind: IMPORT_MODULE,
                    });
                }
                _ => {}
            }
        }
    }

    /// A path as the lexer spells it: `std::path::PathBuf` is `std.path.PathBuf`,
    /// `crate` and `super` keep their word.
    fn rust_module_text(&self, id: u32) -> String {
        match self.kind(id) {
            "use_wildcard" => self.children(id).first().map(|&child| self.rust_module_text(child)).unwrap_or_default(),
            _ => self.text_string(id).replace("::", "."),
        }
    }

    /// Rust's value forms, or `None` for a node the shared arms already read.
    fn rust_form(&mut self, id: u32) -> Option<String> {
        match self.kind(id) {
            "call_expression" => Some(self.rust_call(id)),
            "macro_invocation" => Some(self.rust_macro(id)),
            // `Facts { tag, lines }`: the constructor and its fields.
            "struct_expression" => Some(self.rust_struct_expression(id)),
            "field_initializer_list" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                Some(if items.is_empty() { "0".to_string() } else { self.merge_form(&items) })
            }
            "field_initializer" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.as_slice() {
                    [name, value] => {
                        let key = self.form(*name);
                        let value = self.form(*value);
                        Some(format!("[\"m\",[{key},{value}]]"))
                    }
                    _ => Some("0".to_string()),
                }
            }
            "shorthand_field_initializer" => {
                let name = self.children(id).into_iter().next().map(|child| self.form(child)).unwrap_or_else(|| "0".to_string());
                Some(name)
            }
            "type_cast_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !matches!(self.kind(child), "as")).collect();
                Some(if items.is_empty() { "0".to_string() } else { self.merge_form(&items) })
            }
            // `&x`, `*x`, `x?`, `x.await`: the operator is not a value, the
            // operand is, and it is where the calls are.
            "reference_expression" | "try_expression" | "await_expression" => {
                self.expression_child(id).map(|child| self.form(child))
            }
            "range_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                Some(if items.is_empty() { "0".to_string() } else { self.merge_form(&items) })
            }
            "index_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.split_first() {
                    Some((base, rest)) => {
                        let base = self.form(*base);
                        if rest.is_empty() {
                            Some(base)
                        } else {
                            let mut all = vec![base];
                            all.extend(rest.iter().map(|&item| self.form(item)));
                            Some(format!("[\"m\",[{}]]", all.join(",")))
                        }
                    }
                    None => Some("0".to_string()),
                }
            }
            "tuple_expression" | "array_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                Some(if items.is_empty() { "0".to_string() } else { self.tuple_form(&items) })
            }
            // `|_| self.string()`: the body is what the closure yields.
            "closure_expression" => match self.children(id).into_iter().rev().find(|&child| !self.is_layout(child)) {
                Some(body) if !matches!(self.kind(body), "block") => Some(self.form(body)),
                Some(_) => Some("0".to_string()),
                None => Some("0".to_string()),
            },
            // `value >>= 7` / `bytes += src.len()`: the lexer reads the target
            // as the value, but the right side is still evaluated and its calls
            // are facts — reading only the target dropped `src.len` here.
            "compound_assignment_expr" => {
                let children = self.children(id);
                if let Some(&value) = children.last() {
                    self.form(value);
                }
                Some(children.first().map(|&first| self.form(first)).unwrap_or_else(|| "0".to_string()))
            }
            // A token tree is the macro's argument list; its names and nested
            // calls are values, its punctuation is not.
            "token_tree" => Some(self.rust_tt_value(id)),
            _ => None,
        }
    }

    fn rust_call(&mut self, id: u32) -> String {
        let children = self.children(id);
        let callee = children
            .iter()
            .copied()
            .find(|&child| !matches!(self.kind(child), "arguments" | "type_arguments" | "(" | ")"))
            .unwrap_or(id);
        let (path, receiver) = self.rust_callee(callee);
        let line = self.rust_call_line(callee, id);
        let index = self.push_call(path, line);
        let mut out = format!("[\"c\",{index},{receiver},");
        match children.iter().copied().find(|&child| self.kind(child) == "arguments") {
            Some(list) => out.push_str(&self.arguments(list)),
            None => out.push_str("[]"),
        }
        out.push(']');
        out
    }

    /// The called path and the receiver form: `render` -> (`render`, `0`),
    /// `self.bytes.get` -> (`self.bytes.get`, `["n","self.bytes"]`),
    /// `u32::from` -> (`u32.from`, `0`), `make().run` -> (`.run`, the call's
    /// form).
    fn rust_callee(&mut self, callee: u32) -> (String, String) {
        match self.kind(callee) {
            "field_expression" => {
                let children = self.children(callee);
                let name = self.member_name(callee);
                let Some(&object) = children.first() else {
                    return (format!(".{name}"), "0".to_string());
                };
                if matches!(self.kind(object), "call_expression" | "macro_invocation") {
                    let inner = self.form(object);
                    return (format!(".{name}"), inner);
                }
                let receiver = self.path_of(object);
                // `(value ^ u64::from(b)).wrapping_mul(k)`: the receiver is an
                // expression, and the lexer writes its value there. Reading the
                // call for its facts and then writing 0 as the receiver lost
                // every name in the parenthesised receiver.
                let formed = self.form(object);
                if receiver.is_empty() {
                    (format!(".{name}"), formed)
                } else {
                    let mut text = "[\"n\",".to_string();
                    push_string(&mut text, &receiver);
                    text.push(']');
                    (format!("{receiver}.{name}"), text)
                }
            }
            // `String::from(x)`: the qualifier is the receiver, as the lexer
            // writes it, so a reader can tell `String::from` from `Vec::from`.
            "scoped_identifier" | "scoped_type_identifier" => {
                let path = self.path_of(callee);
                match path.rsplit_once('.') {
                    Some((head, _)) if !head.is_empty() => {
                        let mut text = "[\"n\",".to_string();
                        push_string(&mut text, head);
                        text.push(']');
                        (path, text)
                    }
                    _ => (path, "0".to_string()),
                }
            }
            "generic_function" => {
                let inner = self.children(callee).into_iter().find(|&child| self.kind(child) != "type_arguments");
                match inner {
                    Some(inner) => self.rust_callee(inner),
                    None => (String::new(), "0".to_string()),
                }
            }
            _ => (self.path_of(callee), "0".to_string()),
        }
    }

    /// The line a Rust call is written on: for `a.iter()\n  .map(f)` that is
    /// the line of `map`, which is what a reader points at and where the lexer
    /// stamps it.
    fn rust_call_line(&self, callee: u32, call: u32) -> u32 {
        match self.kind(callee) {
            "field_expression" => {
                let children = self.children(callee);
                if let Some(&name) = children.iter().rev().find(|&&child| {
                    self.node(child).first_child == NONE && !self.is_layout(child)
                }) {
                    return self.line(name);
                }
                self.line(call)
            }
            _ => self.line(callee),
        }
    }

    /// `assert_eq!(a, b)` is a merge of the macro's name and its arguments, not
    /// a call: the lexer reads the name as a name, and a resolver has no callee
    /// called `assert_eq!`.

    /// A macro's token tree, read the way the one-pass lexer reads it: a name
    /// followed by a tree is a call (`assert_eq!(…, shown())` calls `shown`),
    /// everything else is a value. The tokens are not code to the parser, but
    /// they are arguments to a macro that will evaluate them, so the calls in
    /// them are facts.
    fn rust_tt_value(&mut self, id: u32) -> String {
        let kids: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
        let mut parts = Vec::new();
        let mut at = 0;
        while at < kids.len() {
            let node = kids[at];
            let kind = self.kind(node);
            let named = matches!(kind, "identifier" | "scoped_identifier" | "type_identifier");
            if named {
                if let Some(&next) = kids.get(at + 1) {
                    if self.kind(next) == "token_tree" {
                        let path = {
                            let path = self.path_of(node);
                            if path.is_empty() { self.text_string(node) } else { path }
                        };
                        let line = self.line(node);
                        let index = self.push_call(path, line);
                        let args = self.rust_tt_value(next);
                        parts.push(format!("[\"c\",{index},0,[{args}]]"));
                        at += 2;
                        continue;
                    }
                }
            }
            // A literal is a `0` to the lexer, and the guard counts them: the
            // arguments of `assert_eq!(1, 2, "x", f())` are three constants and
            // a call on both sides.
            parts.push(self.form(node));
            at += 1;
        }
        match parts.len() {
            0 => "0".to_string(),
            1 => parts.pop().unwrap_or_else(|| "0".to_string()),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        }
    }

    fn rust_macro(&mut self, id: u32) -> String {
        let mut items = Vec::new();
        for child in self.children(id) {
            match self.kind(child) {
                "token_tree" => items.push(self.form(child)),
                "identifier" | "scoped_identifier" => {
                    let path = self.path_of(child);
                    let mut text = "[\"n\",".to_string();
                    push_string(&mut text, &path);
                    text.push(']');
                    items.push(text);
                }
                _ => {}
            }
        }
        if items.is_empty() {
            "0".to_string()
        } else {
            format!("[\"m\",[{}]]", items.join(","))
        }
    }

    fn rust_struct_expression(&mut self, id: u32) -> String {
        let mut items = Vec::new();
        let children = self.children(id);
        if let Some(&name) = children.first() {
            let path = self.path_of(name);
            if !path.is_empty() {
                let mut text = "[\"n\",".to_string();
                push_string(&mut text, &path);
                text.push(']');
                items.push(text);
            }
        }
        for child in children.iter().skip(1) {
            if self.kind(*child) == "field_initializer_list" {
                items.push(self.form(*child));
            }
        }
        if items.is_empty() {
            "0".to_string()
        } else {
            format!("[\"m\",[{}]]", items.join(","))
        }
    }

    /// Rust's path spellings, or `None` when the shared `path_of` arm is right
    /// (a plain identifier, a literal).
    fn rust_path(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "field_expression" => {
                let name = self.member_name(id);
                let object = self.children(id).first().copied()?;
                let base = self.path_of(object);
                Some(if base.is_empty() { format!(".{name}") } else { format!("{base}.{name}") })
            }
            // `u32::from` is `u32.from`, the lexer's spelling of a path.
            "scoped_identifier" | "scoped_type_identifier" => Some(self.text_string(id).replace("::", ".")),
            "call_expression" | "macro_invocation" => Some(String::new()),
            "struct_expression" => self.children(id).first().map(|&name| self.path_of(name)),
            "generic_function" => self.children(id).into_iter().find(|&child| self.kind(child) != "type_arguments").map(|child| self.path_of(child)),
            _ => None,
        }
    }

    // -- C and C++ ----------------------------------------------------------
    //
    // C and C++ are read here, not through the Python/JS arms above. The
    // declarations are specifier runs over a declarator (the name is inside
    // `direct_declarator`, not a child the shared `Def` can name), a class is
    // `class_specifier`/`struct_or_union_specifier` because the grammar shares
    // one rule between `class`, `struct` and `union`, and a method's owner is
    // the enclosing class or namespace. Everything below is reached only when
    // [`Self::is_c`] is true, so no other language can see it.

    /// The runtime's language name for a C-family grammar. The grammar, not the
    /// lexer tag, is what decides: `CPP` is C++ here and nothing else.
    fn is_c(&self) -> bool {
        matches!(self.language.name, "c" | "cpp")
    }

    /// Nodes C and C++ own. `false` hands the node to the shared walk.
    fn c_function(&mut self, id: u32) {
        let children = self.children(id);
        let declarator = children.iter().copied().find(|&child| self.kind(child) == "direct_declarator");
        let name = declarator
            .and_then(|node| self.first_identifier(node))
            .or_else(|| self.child_of_kind(id, "identifier").map(|node| self.text_string(node)))
            .or_else(|| {
                self.child_of_kind(id, "scoped_type_name")
                    .and_then(|node| self.c_path(node))
                    .and_then(|path| path.rsplit('.').next().map(str::to_string))
            });
        let Some(name) = name else { return };
        let line = self.line(id);
        let params = declarator
            .and_then(|node| self.child_of_kind(node, "parameter_list"))
            .or_else(|| children.iter().copied().find(|&child| self.kind(child) == "parameter_list"))
            .map(|list| self.c_params(list))
            .unwrap_or_default();
        let owner = if self.frame().container { self.frame().class.clone() } else { String::new() };
        let index = self.add_symbol(name.clone(), owner.clone(), KIND_FUNCTION, line, params);
        self.stack.push(Frame { node: id, symbol: index, class: owner, receiver: String::new(), container: false, defines: Some(index) });
    }

    /// A parameter list as the lexer reads it: the last name of each
    /// comma-separated group, with `void` standing for no parameter.
    fn c_params(&self, list: u32) -> Vec<String> {
        let mut out = Vec::new();
        let mut group: Vec<String> = Vec::new();
        for child in self.children(list) {
            if self.kind(child) == "," {
                if let Some(name) = group.pop() {
                    if name != "void" {
                        out.push(name);
                    }
                }
                group.clear();
                continue;
            }
            group.extend(self.c_words_of(child));
        }
        if let Some(name) = group.pop() {
            if name != "void" {
                out.push(name);
            }
        }
        out
    }

    /// `class A : public B {`, `struct s {`, `union u {`: a container whose
    /// members take its name as their owner. A declaration with no body is a
    /// use of the type, not a definition, and the lexer opens no symbol for it.
    fn c_class(&mut self, id: u32) {
        if self.child_of_kind(id, "class_body").is_none() && self.child_of_kind(id, "struct_body").is_none() {
            return;
        }
        let Some(name_node) = self.child_of_kind(id, "identifier") else { return };
        let name = self.text_string(name_node);
        if name.is_empty() {
            return;
        }
        let line = self.line(id);
        let mut bases = Vec::new();
        if let Some(clause) = self.child_of_kind(id, "base_clause") {
            for child in self.children(clause) {
                if matches!(self.kind(child), "plain_type_name" | "scoped_type_name" | "user_type_name") {
                    let path = self.c_path(child).unwrap_or_default();
                    if let Some(last) = path.rsplit('.').next().filter(|last| !last.is_empty()) {
                        bases.push(last.to_string());
                    }
                }
            }
        }
        let index = self.add_symbol(name.clone(), String::new(), KIND_CLASS, line, Vec::new());
        self.facts.symbols[index].bases = bases;
        let symbol = self.frame().symbol;
        self.stack.push(Frame { node: id, symbol, class: name, receiver: String::new(), container: true, defines: Some(index) });
    }

    /// `namespace app { ... }`: the lexer files a namespace as a class, and its
    /// members take the namespace as their owner.
    fn c_namespace(&mut self, id: u32) {
        let Some(name_node) = self.child_of_kind(id, "identifier") else { return };
        let name = self.text_string(name_node);
        let line = self.line(id);
        let index = self.add_symbol(name.clone(), String::new(), KIND_CLASS, line, Vec::new());
        let symbol = self.frame().symbol;
        self.stack.push(Frame { node: id, symbol, class: name, receiver: String::new(), container: true, defines: Some(index) });
    }

    /// `int x = 0;`, `Square box = Square(4);`, `std::string name_;`: the
    /// lexer reads the statement, so a declarator with an initializer is an
    /// assignment event and a declarator without one is the words of the
    /// declaration. `typedef struct {...} T;` is the same shape.
    fn c_words_form(&self, id: u32) -> String {
        self.c_words_form_skipping(id, &[])
    }

    /// The same, without descending into the named subtrees: a struct's body is
    /// its members' statements, not part of the declaration that holds it.
    fn c_words_form_skipping(&self, id: u32, skip: &[&str]) -> String {
        let words = self.c_words_of_skipping(id, skip);
        if words.is_empty() {
            return "0".to_string();
        }
        let mut out = "[\"m\",[".to_string();
        for (index, word) in words.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("[\"n\",");
            push_string(&mut out, word);
            out.push(']');
        }
        out.push_str("]]");
        out
    }

    fn c_words_of(&self, id: u32) -> Vec<String> {
        self.c_words_of_skipping(id, &[])
    }

    fn c_words_of_skipping(&self, id: u32, skip: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            if skip.contains(&self.kind(node)) {
                continue;
            }
            let item = self.node(node);
            if item.first_child == NONE {
                if let Some(name) = self.leaf_name(node) {
                    out.push(name);
                }
                continue;
            }
            let kids = self.children(node);
            for child in kids.into_iter().rev() {
                stack.push(child);
            }
        }
        out
    }

    /// The merge of every value-bearing child, in order.
    fn c_path(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "field_expression" | "qualified_expression" => {
                let name = self.member_name(id);
                let object = self.children(id).into_iter().next()?;
                let base = self.path_of(object);
                Some(if base.is_empty() { format!(".{name}") } else { format!("{base}.{name}") })
            }
            "plain_type_name" | "user_type_name" => {
                let child = self.children(id).into_iter().next()?;
                let path = self.path_of(child);
                (!path.is_empty()).then_some(path)
            }
            "scoped_type_name" => {
                let mut out = String::new();
                for child in self.children(id) {
                    if self.kind(child) == "template_arguments" {
                        continue;
                    }
                    let part = if matches!(self.kind(child), "::") {
                        ".".to_string()
                    } else {
                        self.path_of(child)
                    };
                    if part.is_empty() {
                        continue;
                    }
                    out.push_str(&part);
                }
                (!out.is_empty()).then_some(out)
            }
            _ => None,
        }
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
