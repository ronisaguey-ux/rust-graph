//! Binding a call to a definition. The cascade goes from what the source states
//! to what the tree implies, and stops at a tie: an ambiguous name is reported
//! as unresolved rather than guessed.

use std::collections::HashMap;
use std::rc::Rc;

use crate::facts::{Facts, Import, IMPORT_STAR, KIND_CLASS};
use crate::index::{family, hash, Index, Row};

const MAX_FOLLOW: usize = 8;
const MAX_DEAD_ENDS: usize = 1 << 16;
const CONSTRUCTORS: [&str; 5] = ["__init__", "constructor", "initialize", "init", "new"];
const EXTENSIONS: [&str; 13] = ["", ".py", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".rb", ".lua", ".php", ".pm", ".go"];

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
        Resolver { index, source, dead_methods: DeadEnds::default(), dead_paths: DeadEnds::default() }
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
