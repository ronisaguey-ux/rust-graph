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
//! data (`PY_DEFS`, `JS_DEFS`, `PY_CALLS`, `PY_IMPORTS`), selected by [`kinds`];
//! the walk itself is language neutral. Python, JavaScript, TypeScript and TSX
//! have tables; the other grammars fall back to the module symbol until their
//! table is written.
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
    /// `true` for the brace languages whose declarations are `const x = ...`.
    js: bool,
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
    /// `true` for a container whose members take its name as an owner.
    container: bool,
}

const PY_DEFS: &[Def] = &[
    Def { kind: "class_definition", name: &["identifier"], params: Some("argument_list"), container: true },
    Def { kind: "function_definition", name: &["identifier"], params: Some("parameters"), container: false },
];

const JS_DEFS: &[Def] = &[
    Def { kind: "class_declaration", name: &["identifier"], params: Some("class_heritage"), container: true },
    Def { kind: "class", name: &["identifier"], params: Some("class_heritage"), container: true },
    Def { kind: "interface_declaration", name: &["type_identifier", "identifier"], params: None, container: true },
    Def { kind: "enum_declaration", name: &["identifier", "type_identifier"], params: None, container: true },
    Def { kind: "function_declaration", name: &["identifier"], params: Some("formal_parameters"), container: false },
    Def { kind: "generator_function_declaration", name: &["identifier"], params: Some("formal_parameters"), container: false },
    Def { kind: "method_definition", name: &["property_identifier", "identifier"], params: Some("formal_parameters"), container: false },
    Def { kind: "function_expression", name: &["identifier"], params: Some("formal_parameters"), container: false },
    Def { kind: "arrow_function", name: &[], params: Some("formal_parameters"), container: false },
];

const PY_CALLS: &[&str] = &["call"];
const JS_CALLS: &[&str] = &["call_expression", "new_expression"];
const PY_IMPORTS: &[&str] = &["import_statement", "import_from_statement"];
const JS_IMPORTS: &[&str] = &["import_statement", "export_statement"];

/// Everything the walk needs to know about one language.
fn kinds(lang: Lang) -> Kinds {
    match lang {
        Lang::Python => Kinds { definitions: PY_DEFS, calls: PY_CALLS, imports: PY_IMPORTS, js: false },
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
            Kinds { definitions: JS_DEFS, calls: JS_CALLS, imports: JS_IMPORTS, js: true }
        }
        // The other grammars have no definition table here yet; the walk still
        // returns a record with the module symbol, which is what the caller
        // falls back on. Named so a caller can tell "no facts" from "empty file".
        _ => Kinds { definitions: &[], calls: &[], imports: &[], js: false },
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
    match tag {
        "PY" => Some(Lang::Python),
        "JS" => Some(Lang::JavaScript),
        "TS" => Some(Lang::TypeScript),
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
            stack: vec![Frame { node: tree.root, symbol: 0, class: String::new(), container: false, defines: None }],
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
        let mut child = self.node(id).first_child;
        while child != NONE {
            if self.kind(child) == kind {
                return Some(child);
            }
            child = self.node(child).next_sibling;
        }
        None
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
        // `x` without a keyword list per language.
        if self.kind(id).as_bytes() == text {
            return true;
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

    fn push_call(&mut self, path: String, line: u32) -> usize {
        let symbol = self.frame().symbol;
        let calls = &mut self.facts.symbols[symbol].calls;
        calls.push(Call { path, line });
        calls.len() - 1
    }

    // -- enter / leave ------------------------------------------------------

    fn enter(&mut self, id: u32) {
        let kind = self.kind(id);
        if let Some(def) = self.kinds.definitions.iter().find(|def| def.kind == kind) {
            self.open_definition(id, def);
            return;
        }
        if self.has_kind(id, self.kinds.imports) {
            self.import(id);
            return;
        }
        match kind {
            "expression_statement" => self.expression_statement(id),
            "return_statement" => self.return_statement(id),
            "variable_declarator" if self.is_js() => self.declarator(id),
            "public_field_definition" | "field_definition" | "property_signature" if self.is_js() => {
                self.field(id)
            }
            "throw_statement" | "raise_statement" | "decorator" => self.expression_statement(id),
            "if_statement" => {
                self.condition_before(id);
                self.event("[\"b\"]");
            }
            "elif_clause" | "else_clause" => {
                self.event("[\"e\"]");
                if kind == "elif_clause" {
                    self.condition_before(id);
                }
            }
            "with_statement" => match self.loop_binding(id) {
                Some((targets, value)) => {
                    self.event("[\"b\"]");
                    self.assignment_event(targets, value, self.line(id), false);
                }
                None => {
                    self.condition_before(id);
                    self.event("[\"b\"]");
                }
            },
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
        self.kinds.js
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
            self.stack.push(Frame { node: id, symbol, class: name, container: true, defines: Some(index) });
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
        let mut params = def.params.map(|kind| self.child_of_kind(id, kind)).flatten().map(|node| self.params(id, node)).unwrap_or_default();
        if params.first().is_some_and(|first| matches!(first.as_str(), "self" | "cls" | "this")) {
            params.remove(0);
        }
        let index = self.add_symbol(name, owner.clone(), KIND_FUNCTION, line, params);
        self.stack.push(Frame { node: id, symbol: index, class: owner, container: false, defines: Some(index) });
    }

    /// The name a definition node carries, if any.
    fn def_name(&self, id: u32, def: &Def) -> Option<String> {
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
        let Some(heritage) = self
            .children(id)
            .into_iter()
            .find(|&child| matches!(self.kind(child), "argument_list" | "class_heritage" | "implements_clause"))
        else {
            return out;
        };
        let mut walk = vec![heritage];
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

    /// `def f(a, b=1, *c)`, `function f(a, {b}, ...c)`, `(a, b) => {}`.
    fn params(&self, def: u32, list: u32) -> Vec<String> {
        let mut out = Vec::new();
        let list_kind = self.kind(list);
        // JS class heritage shares the slot but is not a parameter list.
        if list_kind == "class_heritage" {
            return out;
        }
        for child in self.children(list) {
            let kind = self.kind(child);
            match kind {
                "(" | ")" | "," | "[" | "]" | "{" | "}" | ":" => {}
                "parameter" | "identifier" | "required_parameter" | "optional_parameter" | "rest_parameter"
                | "typed_parameter" | "default_parameter" | "typed_default_parameter" | "list_splat_pattern"
                | "dictionary_splat_pattern" => {
                    if let Some(name) = self.first_identifier(child) {
                        out.push(name);
                    }
                }
                "object_pattern" | "array_pattern" => {
                    out.extend(self.pattern_names(child));
                }
                _ => {
                    if let Some(name) = self.first_identifier(child) {
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
            "augmented_assignment" | "augmented_assignment_expression" => self.assignment(child, true),
            _ => {
                let value = self.form(child);
                if value != "0" {
                    let line = self.line(child);
                    self.event(&format!("[\"x\",{line},{value}]"));
                }
            }
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
            "pattern_list" | "tuple_pattern" | "array_pattern" | "list_pattern" | "tuple" | "list" => {
                self.children(id).into_iter().filter_map(|child| self.value_path(child)).collect()
            }
            _ => self.value_path(id).into_iter().collect(),
        }
    }

    fn assignment_event(&mut self, targets: Vec<String>, value: u32, line: u32, augmented: bool) {
        self.assignment_parts(targets, value, line, augmented);
    }

    fn assignment_parts(&mut self, targets: Vec<String>, value: u32, line: u32, augmented: bool) {
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
        let kind = self.kind(value);
        if kind == "new_expression" {
            let path = self.children(value).into_iter().find_map(|child| {
                matches!(self.kind(child), "identifier" | "member_expression" | "attribute").then(|| self.path_of(child))
            })?;
            let last = path.rsplit('.').next().unwrap_or(&path);
            return last.starts_with(|ch: char| ch.is_uppercase()).then_some(last.to_string());
        }
        if kind == "call" || kind == "call_expression" {
            let callee = self.children(value).into_iter().find(|&child| {
                !matches!(self.kind(child), "argument_list" | "arguments" | "type_arguments")
            })?;
            let path = self.path_of(callee);
            let last = path.rsplit('.').next().unwrap_or("").to_string();
            return (last.starts_with(|ch: char| ch.is_uppercase()) && !last.contains('(')).then_some(last);
        }
        None
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

    fn python_import(&mut self, id: u32) {
        let line = self.line(id);
        let children = self.children(id);
        let from = self.kind(id) == "import_from_statement";
        let mut module = String::new();
        let mut seen_from = false;
        let mut index = 0;
        while index < children.len() {
            let child = children[index];
            match self.kind(child) {
                "import" => {
                    if from {
                        seen_from = false;
                    }
                }
                "from" => seen_from = true,
                "dotted_name" | "identifier" if module.is_empty() && (seen_from || !from) => {
                    module = self.text_string(child);
                    if !from {
                        // `import a.b as c` covers the whole rest of the clause.
                        let alias = children
                            .get(index + 2)
                            .filter(|_| children.get(index + 1).is_some_and(|&n| self.kind(n) == "as"))
                            .map(|&n| self.text_string(n))
                            .unwrap_or_else(|| module.clone());
                        self.facts.imports.push(Import { line, module: module.clone(), alias, kind: IMPORT_MODULE, ..Import::default() });
                        module.clear();
                    }
                }
                "*" => {
                    self.facts.imports.push(Import { line, module: module.clone(), kind: IMPORT_STAR, ..Import::default() });
                }
                "aliased_import" => {
                    // `from m import a as b`
                    let inner = self.children(child);
                    let Some(&name_node) = inner.first() else {
                        index += 1;
                        continue;
                    };
                    let name = self.text_string(name_node);
                    let alias = inner.last().filter(|&&last| last != name_node).map(|&last| self.text_string(last)).unwrap_or_else(|| name.clone());
                    self.facts.imports.push(Import { line, module: module.clone(), name, alias, kind: IMPORT_MODULE });
                }
                "dotted_name" | "identifier" => {
                    let name = self.text_string(child);
                    let alias = children
                        .get(index + 2)
                        .filter(|_| children.get(index + 1).is_some_and(|&n| self.kind(n) == "as"))
                        .map(|&n| self.text_string(n))
                        .unwrap_or_else(|| name.clone());
                    self.facts.imports.push(Import { line, module: module.clone(), name, alias, kind: IMPORT_MODULE });
                }
                "as" => {}
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
        if self.kinds.calls.contains(&kind) {
            return self.call_form(id);
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
                    "0".to_string()
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
            "binary_operator" | "comparison_operator" | "boolean_operator" | "binary_expression" | "ternary_expression" => {
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
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.as_slice() {
                    [name, value] => {
                        let mut out = "[\"k\",".to_string();
                        push_string(&mut out, &self.text_string(*name));
                        out.push(',');
                        out.push_str(&self.form(*value));
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
            "string" | "string_literal" | "template_string" | "number" | "integer" | "float" | "true" | "false"
            | "none" | "null" | "undefined" | "regex" | "comment" | "lambda" | "f_string" => "0".to_string(),
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
        let children = self.children(id);
        let is_new = self.kind(id) == "new_expression";
        let callee = children
            .iter()
            .copied()
            .find(|&child| !matches!(self.kind(child), "new" | "argument_list" | "arguments" | "type_arguments" | "(" | ")"))
            .unwrap_or(id);
        // `f().g()`: the receiver is a call, and it is a fact of its own.
        if matches!(self.kind(callee), "attribute" | "member_expression") {
            if let Some(&object) = self.children(callee).first() {
                // `new URL(x).toString()`, `[...xs].filter(f)`: the receiver is
                // an expression that may hold calls of its own.
                self.form(object);
            }
        }
        let (path, receiver) = self.call_target(callee);
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
        out
    }

    /// The called path and the receiver form: `f(x)` -> (`f`, `0`),
    /// `a.b.c(x)` -> (`a.b.c`, `["n","a.b"]`).
    fn call_target(&self, callee: u32) -> (String, String) {
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
                let receiver = if receiver_path.is_empty() {
                    "0".to_string()
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &receiver_path);
                    out.push(']');
                    out
                };
                (path, receiver)
            }
            "call" | "call_expression" => {
                // `f()(...)`: no name to call, so the path is the callee's and
                // the receiver is unknown.
                let (path, _) = self.call_target(callee);
                (path, "0".to_string())
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
            out.push_str(&self.form(child));
        }
        out.push(']');
        out
    }

    /// A dotted name off the source: `a . b` and `a.b` are the same path, and
    /// `this` becomes `self` as the flow form spells it.
    fn path_of(&self, id: u32) -> String {
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
            _ => String::new(),
        }
    }

    /// The path of a value used as an assignment target, or empty when the
    /// target is not a name (a literal index, a call).
    fn value_path(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "identifier" | "attribute" | "member_expression" | "subscript" | "subscript_expression" | "this" => {
                let path = self.path_of(id);
                (!path.is_empty()).then_some(path)
            }
            "tuple_pattern" | "pattern_list" | "array_pattern" | "list_pattern" | "object_pattern" => None,
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
