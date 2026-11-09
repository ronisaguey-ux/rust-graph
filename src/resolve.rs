//! Binding a call to a definition. The cascade goes from what the source states
//! to what the tree implies, and stops at a tie: an ambiguous name is reported
//! as unresolved rather than guessed.

use std::collections::HashMap;
use std::rc::Rc;

use crate::facts::{Facts, Import, Symbol, IMPORT_STAR, KIND_CLASS, KIND_FUNCTION};
use crate::index::{family, hash, Index, Row};

const MAX_FOLLOW: usize = 8;
const MAX_RECEIVERS: usize = 4;
const MAX_DEAD_ENDS: usize = 1 << 16;
const CONSTRUCTORS: [&str; 5] = ["__init__", "constructor", "initialize", "init", "new"];
const EXTENSIONS: [&str; 13] = ["", ".py", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".rb", ".lua", ".php", ".pm", ".go"];

/// The flow encoding, read back. A symbol's `events` is a comma-separated list
/// of forms (`["c",index,receiver,args]`, `["r",line,value]`, `["n","a.b"]`,
/// ...) without its outer brackets; the resolver needs exactly one field of it
/// — which call produced a call's receiver — so a strict reader of the shape is
/// enough and no dependency is pulled in for the rest.
mod form {
    #[derive(Debug)]
    pub enum Value {
        Null,
        Bool,
        Num(u32),
        Str(String),
        List(Vec<Value>),
    }

    pub fn parse(text: &str) -> Option<Value> {
        let mut reader = Reader { bytes: text.as_bytes(), pos: 0 };
        let value = reader.value()?;
        reader.space();
        (reader.pos == reader.bytes.len()).then_some(value)
    }

    struct Reader<'a> {
        bytes: &'a [u8],
        pos: usize,
    }

    impl Reader<'_> {
        fn space(&mut self) {
            while matches!(self.bytes.get(self.pos), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                self.pos += 1;
            }
        }

        fn value(&mut self) -> Option<Value> {
            self.space();
            match *self.bytes.get(self.pos)? {
                b'[' => self.list(),
                b'"' => self.string().map(Value::Str),
                b't' => self.word("true", Value::Bool),
                b'f' => self.word("false", Value::Bool),
                b'n' => self.word("null", Value::Null),
                b'-' | b'0'..=b'9' => self.number(),
                _ => None,
            }
        }

        fn word(&mut self, text: &str, value: Value) -> Option<Value> {
            self.bytes.get(self.pos..self.pos + text.len())?.eq(text.as_bytes()).then(|| {
                self.pos += text.len();
                value
            })
        }

        fn number(&mut self) -> Option<Value> {
            let start = self.pos;
            if self.bytes.get(self.pos) == Some(&b'-') {
                // No form the resolver reads is negative; a line or an index is
                // never one, so a negative number is not the shape to read.
                return None;
            }
            while matches!(self.bytes.get(self.pos), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
            let text = std::str::from_utf8(self.bytes.get(start..self.pos)?).ok()?;
            let value = text.parse::<u32>().ok()?;
            // A fractional or exponent part is not a form this reader claims.
            matches!(self.bytes.get(self.pos), None | Some(b',' | b']' | b' ' | b'\t' | b'\n' | b'\r')).then_some(Value::Num(value))
        }

        fn list(&mut self) -> Option<Value> {
            self.pos += 1;
            let mut items = Vec::new();
            self.space();
            if self.bytes.get(self.pos) == Some(&b']') {
                self.pos += 1;
                return Some(Value::List(items));
            }
            loop {
                items.push(self.value()?);
                self.space();
                match *self.bytes.get(self.pos)? {
                    b',' => self.pos += 1,
                    b']' => {
                        self.pos += 1;
                        return Some(Value::List(items));
                    }
                    _ => return None,
                }
            }
        }

        fn string(&mut self) -> Option<String> {
            self.pos += 1;
            let mut out = String::new();
            loop {
                let byte = *self.bytes.get(self.pos)?;
                self.pos += 1;
                match byte {
                    b'"' => return Some(out),
                    b'\\' => {
                        let escape = *self.bytes.get(self.pos)?;
                        self.pos += 1;
                        match escape {
                            b'"' => out.push('"'),
                            b'\\' => out.push('\\'),
                            b'/' => out.push('/'),
                            b'n' => out.push('\n'),
                            b'r' => out.push('\r'),
                            b't' => out.push('\t'),
                            b'u' => {
                                let hex = self.bytes.get(self.pos..self.pos + 4)?;
                                self.pos += 4;
                                let code = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                                out.push(char::from_u32(code)?);
                            }
                            _ => return None,
                        }
                    }
                    byte if byte < 0x20 => return None,
                    // Only the ASCII forms are read (a tag, a call index); a
                    // multi-byte name is copied byte by byte and never compared.
                    byte => out.push(char::from(byte)),
                }
            }
        }
    }
}

/// For each call a symbol records, the call whose result it is a method call
/// on, when the walk recorded one: `A().run` holds `0` at the index of `.run`,
/// and `None` everywhere the receiver is not a call.
///
/// The stored field is the inside of the array — the pipeline writes the
/// brackets around it — so it is wrapped before it is read.
pub fn receiver_calls(events: &str) -> Vec<Option<u32>> {
    let mut out: Vec<Option<u32>> = Vec::new();
    if events.is_empty() {
        return out;
    }
    if let Some(value) = form::parse(&format!("[{events}]")) {
        collect_receivers(&value, &mut out);
    }
    out
}

fn collect_receivers(value: &form::Value, out: &mut Vec<Option<u32>>) {
    let form::Value::List(items) = value else { return };
    if matches!(items.first(), Some(form::Value::Str(tag)) if tag == "c") {
        if let Some(form::Value::Num(index)) = items.get(1) {
            let receiver = match items.get(2) {
                Some(form::Value::List(inner)) => match (inner.first(), inner.get(1)) {
                    (Some(form::Value::Str(tag)), Some(form::Value::Num(call))) if tag == "c" => Some(*call),
                    _ => None,
                },
                _ => None,
            };
            let index = *index as usize;
            if out.len() <= index {
                out.resize(index + 1, None);
            }
            out[index] = receiver;
        }
    }
    // A call form appears where it is used: at a statement, in an argument, or
    // as another call's receiver, which carries that receiver's own receiver.
    for item in items {
        collect_receivers(item, out);
    }
}

/// The value of each `return`, as the call index it is, or `None` when the
/// return is not a call at all (a bare `return`, a name, a literal).
fn returns_of(events: &str) -> Vec<Option<u32>> {
    let mut out = Vec::new();
    if events.is_empty() {
        return out;
    }
    let Some(form::Value::List(items)) = form::parse(&format!("[{events}]")) else {
        return out;
    };
    for item in &items {
        let form::Value::List(parts) = item else { continue };
        if !matches!(parts.first(), Some(form::Value::Str(tag)) if tag == "r") {
            continue;
        }
        let value = match parts.get(2) {
            Some(form::Value::List(inner)) => match (inner.first(), inner.get(1)) {
                (Some(form::Value::Str(tag)), Some(form::Value::Num(call))) if tag == "c" => Some(*call),
                _ => None,
            },
            _ => None,
        };
        out.push(value);
    }
    out
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Tag {
    Extracted,
    Inferred,
}

impl Tag {
    pub fn text(self) -> &'static str {
        match self {
            Tag::Extracted => "EXTRACTED",
            Tag::Inferred => "INFERRED",
        }
    }
}

pub struct Bound {
    pub row: Row,
    pub tag: Tag,
    pub strategy: &'static str,
    pub constructor: bool,
}

pub enum Outcome {
    Bound(Bound),
    /// Bound by an import to something outside the tree: its canonical path.
    External(String),
    Ambiguous,
    Unknown,
}

/// Imports of other files, read from the pack when a chain leads through them.
/// What a bare name can reach in this language.
#[derive(Clone, Copy, Default)]
pub struct Scope {
    pub globals: bool,
    pub members: bool,
}

pub trait ImportSource {
    fn imports_of(&mut self, file: u32) -> Rc<Vec<Import>>;
    fn bases_of(&mut self, file: u32, class: &str) -> Vec<String>;
}

/// Searches that found nothing, each with the shallowest depth it failed from.
///
/// Cyclic bases and re-exports make the walk branch at every step up to
/// `MAX_FOLLOW`, which is `branches^8` lookups without this. A plain visited set
/// would stop that but change answers: a class first met at the depth cap can
/// still succeed from a shorter path. A failure does transfer the other way,
/// since a deeper start only has less of the same walk left, so skipping those
/// leaves every result as it was. Forgetting an entry costs time, never accuracy.
#[derive(Default)]
struct DeadEnds(HashMap<(u32, u64, u64), usize>);

impl DeadEnds {
    fn covers(&self, key: (u32, u64, u64), depth: usize) -> bool {
        self.0.get(&key).is_some_and(|seen| *seen <= depth)
    }

    fn record(&mut self, key: (u32, u64, u64), depth: usize) {
        if self.0.len() >= MAX_DEAD_ENDS {
            self.0.clear();
        }
        let seen = self.0.entry(key).or_insert(depth);
        *seen = depth.min(*seen);
    }
}

pub struct Resolver<'a, S: ImportSource> {
    index: &'a Index,
    source: &'a mut S,
    dead_methods: DeadEnds,
    dead_paths: DeadEnds,
    /// Per symbol, the call each method call's receiver is: read from the
    /// symbol's flow events once, keyed by the symbol's key.
    receivers: HashMap<(u32, u64), Rc<Vec<Option<u32>>>>,
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |pair| pair.0)
}

fn join(dir: &str, relative: &str) -> Option<String> {
    let mut parts: Vec<&str> = if dir.is_empty() { Vec::new() } else { dir.split('/').collect() };
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

impl<'a, S: ImportSource> Resolver<'a, S> {
    pub fn new(index: &'a Index, source: &'a mut S) -> Self {
        Resolver { index, source, dead_methods: DeadEnds::default(), dead_paths: DeadEnds::default(), receivers: HashMap::new() }
    }

    fn exact(&self, stem_path: &str, file: u32) -> Option<u32> {
        let family = self.index.families[file as usize];
        let last = stem_path.rsplit('/').next()?;
        let last = last.split('.').next().unwrap_or(last);
        let mut hits = self.index.candidates(last).iter().copied().filter(|candidate| {
            self.index.families[*candidate as usize] == family && {
                let path = self.index.paths[*candidate as usize].as_str();
                EXTENSIONS.iter().any(|ext| {
                    let direct = format!("{stem_path}{ext}");
                    path == direct
                        || (!ext.is_empty()
                            && path.strip_prefix(stem_path).is_some_and(|rest| {
                                rest.strip_prefix('/').is_some_and(|name| {
                                    name.strip_suffix(ext).is_some_and(|stem| matches!(stem, "__init__" | "index" | "mod"))
                                })
                            }))
                })
            }
        });
        hits.next()
    }

    /// A path written from the tree root or any prefix of it (`src/` layouts).
    fn by_suffix(&self, module_path: &str, file: u32) -> Option<u32> {
        let family = self.index.families[file as usize];
        let last = module_path.rsplit('/').next()?;
        let wanted = format!("/{module_path}");
        let mut found = None;
        for candidate in self.index.candidates(last.split('.').next().unwrap_or(last)) {
            if self.index.families[*candidate as usize] != family {
                continue;
            }
            let path = self.index.paths[*candidate as usize].as_str();
            let stem = path.rsplit_once('.').map_or(path, |pair| pair.0);
            let package = stem.rsplit_once('/').filter(|pair| matches!(pair.1, "__init__" | "index" | "mod")).map_or(stem, |pair| pair.0);
            if stem.ends_with(&wanted) || package.ends_with(&wanted) || path.ends_with(&wanted) {
                if found.is_some() {
                    return None;
                }
                found = Some(*candidate);
            }
        }
        found
    }

    fn module_file(&self, module: &str, from: u32) -> Option<u32> {
        let importer = self.index.paths[from as usize].as_str();
        let dir = parent(importer);
        if module.starts_with("./") || module.starts_with("../") {
            return self.exact(&join(dir, module)?, from);
        }
        if module.starts_with('.') {
            let level = module.bytes().take_while(|byte| *byte == b'.').count();
            let mut base = dir.to_string();
            for _ in 1..level {
                base = parent(&base).to_string();
            }
            let tail = module[level..].replace('.', "/");
            let target = match (base.is_empty(), tail.is_empty()) {
                (_, true) => base,
                (true, false) => tail,
                (false, false) => format!("{base}/{tail}"),
            };
            return if target.is_empty() { None } else { self.exact(&target, from) };
        }
        let is_c_family = self.index.families[from as usize] == family("C");
        if module.contains('/') || is_c_family || module.ends_with(".rb") || module.ends_with(".php") {
            if let Some(hit) = join(dir, module).and_then(|path| self.exact(&path, from)) {
                return Some(hit);
            }
            return self.exact(module, from).or_else(|| self.by_suffix(module, from));
        }
        let path = module.replace("::", "/").replace('.', "/");
        let sibling = join(dir, &path).and_then(|target| self.exact(&target, from));
        self.exact(&path, from).or(sibling).or_else(|| self.by_suffix(&path, from))
    }

    /// `names` looked up in a file, following what that file re-exports.
    fn in_file(&mut self, file: u32, names: &[&str], depth: usize) -> Option<(Row, bool)> {
        let (&first, rest) = names.split_first()?;
        if let Some(row) = self.index.top_level(file, hash(first)) {
            if row.kind == KIND_CLASS {
                if let Some(&method) = rest.first() {
                    return self.method(file, first, method, depth).map(|row| (row, false));
                }
                return Some(self.construct(row, file, first));
            }
            return rest.is_empty().then_some((row, false));
        }
        if depth >= MAX_FOLLOW {
            return None;
        }
        let imports = self.source.imports_of(file);
        if imports.is_empty() {
            return None;
        }
        let path = names.iter().fold(0, |path: u64, name| path.rotate_left(7) ^ hash(name));
        let key = (file, path, names.len() as u64);
        if self.dead_paths.covers(key, depth) {
            return None;
        }
        for import in imports.iter().filter(|import| import.alias == first && import.kind != IMPORT_STAR) {
            if let Some(found) = self.through(import, rest, file, depth + 1) {
                return Some(found);
            }
        }
        for import in imports.iter().filter(|import| import.kind == IMPORT_STAR) {
            if let Some(target) = self.module_file(&import.module, file) {
                if let Some(found) = self.in_file(target, names, depth + 1) {
                    return Some(found);
                }
            }
        }
        self.dead_paths.record(key, depth);
        None
    }

    fn construct(&self, class: Row, file: u32, name: &str) -> (Row, bool) {
        let owner = hash(name);
        for word in CONSTRUCTORS {
            if let Some(row) = self.index.member(file, owner, hash(word)) {
                return (row, true);
            }
        }
        (class, true)
    }

    fn method(&mut self, file: u32, class: &str, name: &str, depth: usize) -> Option<Row> {
        let key = (file, hash(class), hash(name));
        if let Some(row) = self.index.member(file, key.1, key.2) {
            return Some(row);
        }
        if depth >= MAX_FOLLOW || self.dead_methods.covers(key, depth) {
            return None;
        }
        let bases = self.source.bases_of(file, class);
        if bases.is_empty() {
            return None;
        }
        for base in bases {
            if self.index.class_in(file, hash(&base)) {
                if let Some(row) = self.method(file, &base, name, depth + 1) {
                    return Some(row);
                }
                continue;
            }
            let imports = self.source.imports_of(file);
            for import in imports.iter().filter(|import| import.alias == base) {
                if let Some((owner_file, class_name)) = self.class_through(import, file) {
                    if let Some(row) = self.method(owner_file, &class_name, name, depth + 1) {
                        return Some(row);
                    }
                }
            }
        }
        self.dead_methods.record(key, depth);
        None
    }

    fn class_through(&mut self, import: &Import, from: u32) -> Option<(u32, String)> {
        let target = self.module_file(&import.module, from)?;
        let name = if import.name.is_empty() { import.alias.clone() } else { import.name.clone() };
        self.index.class_in(target, hash(&name)).then_some((target, name))
    }

    /// Follow one import, with `rest` the path still to apply after it.
    fn through(&mut self, import: &Import, rest: &[&str], from: u32, depth: usize) -> Option<(Row, bool)> {
        let mut names: Vec<&str> = Vec::with_capacity(rest.len() + 1);
        if !import.name.is_empty() {
            names.push(&import.name);
        }
        names.extend_from_slice(rest);
        if let Some(target) = self.module_file(&import.module, from) {
            if let Some(found) = self.in_file(target, &names, depth) {
                return Some(found);
            }
        }
        // `from app import dangerous` and `use a::b::helper` may name a module.
        let joiner = if import.module.contains('/') { "/" } else { "." };
        let mut module = import.module.clone();
        let mut at = 0;
        while at < names.len() {
            if !module.is_empty() && !module.ends_with('.') {
                module.push_str(joiner);
            }
            module.push_str(names[at]);
            at += 1;
            if let Some(target) = self.module_file(&module, from) {
                if let Some(found) = self.in_file(target, &names[at..], depth) {
                    return Some(found);
                }
            }
        }
        // `use a::b::helper`: the last part of the path is the member.
        if import.name.is_empty() {
            if let Some((prefix, last)) = import.module.rsplit_once(['.', ':']) {
                let prefix = prefix.trim_end_matches(':');
                if !prefix.is_empty() {
                    if let Some(target) = self.module_file(prefix, from) {
                        let mut path = vec![last];
                        path.extend_from_slice(rest);
                        return self.in_file(target, &path, depth);
                    }
                }
            }
        }
        None
    }

    fn typed_method(&mut self, file: u32, facts: &Facts, var: &str, name: &str) -> Option<(Row, &'static str)> {
        let class = facts.vars.iter().rev().find(|pair| pair.0 == var).map(|pair| pair.1.clone())?;
        self.method_of_class(file, facts, &class, name)
    }

    fn method_of_class(&mut self, file: u32, facts: &Facts, class: &str, name: &str) -> Option<(Row, &'static str)> {
        if self.index.class_in(file, hash(class)) {
            return self.method(file, class, name, 0).map(|row| (row, "typed-variable"));
        }
        for import in facts.imports.iter().filter(|import| import.alias == class) {
            if let Some((owner_file, class_name)) = self.class_through(import, file) {
                if let Some(row) = self.method(owner_file, &class_name, name, 0) {
                    return Some((row, "typed-variable"));
                }
            }
            if let Some((row, _)) = self.through(import, &[name], file, 0) {
                return Some((row, "typed-variable"));
            }
        }
        None
    }

    /// The write-name of a class row: `A` for both `A` and `A.__init__`.
    fn class_name(&self, row: &Row) -> Option<String> {
        let id = self.index.id(row);
        let key = id.rsplit_once(':')?.1;
        Some(key.split('.').next()?.to_string())
    }

    /// The owner and the name a row was written under: `A.run` is `("A", "run")`,
    /// a class `A` is `("", "A")` and a module-level `make` is `("", "make")`.
    /// The key is the only place a row's own name survives the index (the row
    /// holds its hashes), so it is read from there.
    fn row_parts(&self, row: &Row) -> Option<(String, String)> {
        let id = self.index.id(row);
        let key = id.rsplit_once(':')?.1;
        let (owner, name) = key.rsplit_once('.').unwrap_or(("", key));
        Some((owner.to_string(), name.to_string()))
    }

    /// The call that produced this call's receiver, when the walk recorded it.
    fn receiver_call(&mut self, file: u32, symbol: &Symbol, position: usize) -> Option<u32> {
        let key = (file, hash(&symbol.key));
        if !self.receivers.contains_key(&key) {
            let table = receiver_calls(&symbol.events);
            self.receivers.insert(key, Rc::new(table));
        }
        self.receivers.get(&key)?.get(position).copied().flatten()
    }

    /// The class whose instance the call at `position` produces, and the
    /// strategy that says how it was read. `None` is a value whose class the
    /// tree does not state.
    fn call_class(
        &mut self,
        symbol: &Symbol,
        position: usize,
        file: u32,
        facts: &Facts,
        scope: Scope,
        depth: usize,
    ) -> Option<(u32, String, &'static str)> {
        if depth > MAX_RECEIVERS {
            return None;
        }
        let path = symbol.calls.get(position)?.path.clone();
        if let Some(name) = path.strip_prefix('.') {
            // `.b` on a receiver that is itself a call: `A().b().c()`. What
            // `.c` is called on is whatever `A.b` returns, and only `A.b`'s
            // body can say that.
            if name.is_empty() || name.contains('.') {
                return None;
            }
            let receiver = self.receiver_call(file, symbol, position)?;
            let (class_file, class, _) = self.call_class(symbol, receiver as usize, file, facts, scope, depth + 1)?;
            let method = self.method(class_file, &class, name, 0)?;
            let (owner_file, owner) = self.returned_from_row(facts, &method, file, scope, depth + 1)?;
            return Some((owner_file, owner, "receiver-return"));
        }
        match self.call(&path, file, &symbol.owner, facts, scope) {
            // `A(...)`, or `A` with no constructor of its own: the class is
            // the row's own name or the owner of the constructor it found.
            Outcome::Bound(bound) if bound.row.kind == KIND_CLASS || bound.constructor => {
                Some((bound.row.file, self.class_name(&bound.row)?, "receiver-call"))
            }
            // `make()`: a function produces an instance only when its body
            // says so, and the body is its own row, not the caller's.
            Outcome::Bound(bound) => {
                let (owner_file, class) = self.returned_from_row(facts, &bound.row, file, scope, depth + 1)?;
                Some((owner_file, class, "receiver-return"))
            }
            _ => None,
        }
    }

    /// `returned_class` for a resolved row, with the owner and the name read
    /// from the row itself: `self.make()` is `A.make` whatever symbol calls it,
    /// and `obj.make()` at module level is still `A.make`.
    fn returned_from_row(
        &mut self,
        facts: &Facts,
        row: &Row,
        file: u32,
        scope: Scope,
        depth: usize,
    ) -> Option<(u32, String)> {
        // The body is only readable in the file being resolved; a function in
        // another file needs that file's facts, which this pass does not hold.
        if row.file != file {
            return None;
        }
        let (owner, name) = self.row_parts(row)?;
        self.returned_class(facts, &owner, &name, file, scope, depth)
    }

    /// The class a same-file callable returns, when every `return` in its body
    /// is a call to the same class: `def chain(self): return A()`. A bare
    /// `return`, a second class, a name or a literal leaves it undecided.
    fn returned_class(
        &mut self,
        facts: &Facts,
        owner: &str,
        name: &str,
        file: u32,
        scope: Scope,
        depth: usize,
    ) -> Option<(u32, String)> {
        if depth > MAX_RECEIVERS {
            return None;
        }
        let symbol = facts
            .symbols
            .iter()
            .find(|symbol| symbol.name == name && symbol.owner == owner && symbol.kind == KIND_FUNCTION)?;
        let returns = returns_of(&symbol.events);
        if returns.is_empty() {
            return None;
        }
        let mut found: Option<(u32, String)> = None;
        for index in returns {
            let index = index?;
            let path = symbol.calls.get(index as usize)?.path.clone();
            // A method call's value is not a constructor call; only a name is.
            if path.contains('.') {
                return None;
            }
            let bound = match self.call(&path, file, owner, facts, scope) {
                Outcome::Bound(bound) if bound.row.kind == KIND_CLASS || bound.constructor => bound,
                _ => return None,
            };
            let class = (bound.row.file, self.class_name(&bound.row)?);
            match &found {
                Some(seen) if *seen != class => return None,
                _ => found = Some(class),
            }
        }
        found
    }

    /// `call`, with the call's position in its symbol so the receiver the walk
    /// recorded can be read. `.run` on `A()` is tied to `A.run` exactly as the
    /// one-pass lexer's glued `A().run` is; the path here keeps the name the
    /// source spells, and the link is a fact of its own.
    pub fn call_at(
        &mut self,
        path: &str,
        symbol: &Symbol,
        position: usize,
        file: u32,
        facts: &Facts,
        scope: Scope,
    ) -> Outcome {
        if let Some(name) = path.strip_prefix('.') {
            if !name.is_empty() && !name.contains('.') {
                if let Some(receiver) = self.receiver_call(file, symbol, position) {
                    if let Some((class_file, class, strategy)) =
                        self.call_class(symbol, receiver as usize, file, facts, scope, 0)
                    {
                        if let Some(row) = self.method(class_file, &class, name, 0) {
                            return Outcome::Bound(Bound { row, tag: Tag::Inferred, strategy, constructor: false });
                        }
                    }
                }
            }
            return Outcome::Unknown;
        }
        self.call(path, file, &symbol.owner, facts, scope)
    }

    /// The tie-breaks for languages whose names are visible without an import.
    fn global(&self, file: u32, name: &str) -> Outcome {
        let family = self.index.families[file as usize];
        let rows: Vec<Row> = self
            .index
            .named(hash(name))
            .into_iter()
            .filter(|row| self.index.families[row.file as usize] == family && row.kind != KIND_CLASS)
            .collect();
        let dir = parent(&self.index.paths[file as usize]);
        let near: Vec<&Row> = rows.iter().filter(|row| parent(&self.index.paths[row.file as usize]) == dir).collect();
        match (near.as_slice(), rows.as_slice()) {
            ([one], _) => Outcome::Bound(Bound { row: **one, tag: Tag::Inferred, strategy: "same-directory", constructor: false }),
            (_, [one]) => Outcome::Bound(Bound { row: *one, tag: Tag::Inferred, strategy: "unique-name", constructor: false }),
            (_, []) => Outcome::Unknown,
            _ => Outcome::Ambiguous,
        }
    }

    pub fn call(&mut self, path: &str, file: u32, class: &str, facts: &Facts, scope: Scope) -> Outcome {
        if path.starts_with('.') {
            return Outcome::Unknown;
        }
        if let Some((class, method)) = path.split_once("().") {
            let class = class.rsplit('.').next().unwrap_or(class);
            return match self.method_of_class(file, facts, class, method) {
                Some((row, strategy)) => Outcome::Bound(Bound { row, tag: Tag::Inferred, strategy, constructor: false }),
                None => Outcome::Unknown,
            };
        }
        let parts: Vec<&str> = path.split('.').collect();
        let bound = |row: Row, tag: Tag, strategy: &'static str, constructor: bool| {
            Outcome::Bound(Bound { row, tag, strategy, constructor })
        };
        if parts[0] == "self" {
            if parts.len() == 2 && !class.is_empty() {
                return match self.method(file, class, parts[1], 0) {
                    Some(row) => bound(row, Tag::Extracted, "same-class", false),
                    None => Outcome::Unknown,
                };
            }
            if parts.len() == 3 {
                let var = format!("self.{}", parts[1]);
                if let Some((row, strategy)) = self.typed_method(file, facts, &var, parts[2]) {
                    return bound(row, Tag::Inferred, strategy, false);
                }
            }
            return Outcome::Unknown;
        }
        if parts.len() == 1 && scope.members && !class.is_empty() {
            if let Some(row) = self.method(file, class, parts[0], 0) {
                return bound(row, Tag::Extracted, "same-class", false);
            }
        }
        if parts.len() == 1 {
            if let Some(row) = self.index.top_level(file, hash(parts[0])) {
                if row.kind == KIND_CLASS {
                    let (row, constructor) = self.construct(row, file, parts[0]);
                    return bound(row, Tag::Extracted, "same-file", constructor);
                }
                return bound(row, Tag::Extracted, "same-file", false);
            }
        }
        // The longest import binding that prefixes the path wins.
        let mut best: Option<(&Import, usize)> = None;
        for import in facts.imports.iter().filter(|import| import.kind != IMPORT_STAR) {
            let width = import.alias.split('.').count();
            if width <= parts.len()
                && import.alias.split('.').zip(&parts).all(|(left, right)| left == *right)
                && best.is_none_or(|(_, seen)| width > seen)
            {
                best = Some((import, width));
            }
        }
        if let Some((import, width)) = best {
            if let Some((row, constructor)) = self.through(import, &parts[width..], file, 0) {
                return bound(row, Tag::Extracted, "import", constructor);
            }
            let mut canonical = import.module.trim_start_matches("./").replace('/', ".").replace("::", ".");
            if !import.name.is_empty() {
                canonical.push('.');
                canonical.push_str(&import.name);
            }
            for part in &parts[width..] {
                canonical.push('.');
                canonical.push_str(part);
            }
            return Outcome::External(canonical);
        }
        if parts.len() == 2 {
            if let Some((row, strategy)) = self.typed_method(file, facts, parts[0], parts[1]) {
                return bound(row, Tag::Inferred, strategy, false);
            }
            if self.index.class_in(file, hash(parts[0])) {
                if let Some(row) = self.method(file, parts[0], parts[1], 0) {
                    return bound(row, Tag::Extracted, "same-file", false);
                }
            }
        }
        let stars: Vec<Import> = facts.imports.iter().filter(|import| import.kind == IMPORT_STAR).cloned().collect();
        for import in &stars {
            if let Some(target) = self.module_file(&import.module, file) {
                if let Some((row, constructor)) = self.in_file(target, &parts, 0) {
                    return bound(row, Tag::Extracted, "star-import", constructor);
                }
            }
        }
        if parts.len() == 1 && scope.globals {
            return self.global(file, parts[0]);
        }
        Outcome::Unknown
    }
}

/// Where an import statement points, for the `imports` section.
pub fn import_target<S: ImportSource>(resolver: &Resolver<'_, S>, import: &Import, file: u32) -> Option<String> {
    resolver.module_file(&import.module, file).map(|target| resolver.index.paths[target as usize].clone())
}

/// A small read-through cache so a barrel file is decoded once per run of files.
pub struct Cached<T> {
    pub entries: HashMap<u32, Rc<T>>,
    limit: usize,
}

impl<T> Cached<T> {
    pub fn new(limit: usize) -> Self {
        Cached { entries: HashMap::new(), limit }
    }

    pub fn get_or(&mut self, key: u32, load: impl FnOnce() -> T) -> Rc<T> {
        if self.entries.len() >= self.limit && !self.entries.contains_key(&key) {
            self.entries.clear();
        }
        self.entries.entry(key).or_insert_with(|| Rc::new(load())).clone()
    }
}
