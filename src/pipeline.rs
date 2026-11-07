//! Two streaming passes. Pass one reads each file once and appends its facts to
//! the pack, copying the old record when the SHA-256 matches. Pass two reads the
//! pack a file at a time, resolves calls, and writes each section to its own
//! spool; the spools are then concatenated into one sorted document.

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use crate::facts::{Facts, Import, Symbol, IMPORT_STAR, KIND_CLASS, KIND_MODULE};
use crate::index::Index;
use crate::json::{push_list, push_optional, push_string};
use crate::langs::{self, Language};
use crate::parser;
use crate::resolve::{import_target, Cached, ImportSource, Outcome, Resolver, Tag, Scope};
use crate::sha256;

const SKIPPED_DIRS: [&str; 14] = [
    ".git", ".hg", ".svn", ".rust-graph", "node_modules", "target", "__pycache__", ".venv", "venv", ".tox", ".mypy_cache",
    ".pytest_cache", ".gradle", ".idea",
];
const PACK_MAGIC: &[u8; 8] = b"RGPACK01";
/// Bump when a parser change alters what a record holds: old records are void.
const FACTS_VERSION: u32 = 1;
const TOP_LIST: usize = 15;

pub struct Options {
    pub root: PathBuf,
    pub out: Option<PathBuf>,
    pub stdout: bool,
    pub cache: Option<PathBuf>,
    pub no_cache: bool,
    pub max_files: usize,
    pub max_file_bytes: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            root: PathBuf::new(),
            out: None,
            stdout: false,
            cache: None,
            no_cache: false,
            max_files: 200_000,
            max_file_bytes: 2 * 1024 * 1024,
        }
    }
}

struct Entry {
    path: String,
    sha: [u8; 32],
    bytes: u64,
    offset: u64,
    length: u32,
}

#[derive(Default)]
struct Totals {
    parsed: usize,
    reused: usize,
    too_large: usize,
    truncated: bool,
    bytes: u64,
}

fn suffix_of(name: &str) -> Option<String> {
    name.rsplit_once('.').map(|pair| pair.1.to_ascii_lowercase())
}

fn known_suffix(suffix: &str) -> bool {
    matches!(suffix, "h" | "m" | "pl") || langs::LANGUAGES.iter().any(|language| language.extensions.contains(&suffix))
}

fn walk(root: &Path, relative: &str, out: &mut Vec<String>) -> io::Result<()> {
    let directory = if relative.is_empty() { root.to_path_buf() } else { root.join(relative) };
    let mut names: Vec<(String, bool)> = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        // Symlinks can leave the tree; opening a FIFO or a device can block forever.
        if !kind.is_file() && !kind.is_dir() {
            continue;
        }
        if let Ok(name) = entry.file_name().into_string() {
            names.push((name, kind.is_dir()));
        }
    }
    names.sort_unstable();
    for (name, is_dir) in names {
        let path = if relative.is_empty() { name.clone() } else { format!("{relative}/{name}") };
        if is_dir {
            if !SKIPPED_DIRS.contains(&name.as_str()) {
                walk(root, &path, out)?;
            }
        } else if suffix_of(&name).is_some_and(|suffix| known_suffix(&suffix)) {
            out.push(path);
        }
    }
    Ok(())
}

fn language_of(path: &str, bytes: &[u8]) -> Option<&'static Language> {
    langs::by_content(&suffix_of(path)?, bytes)
}

struct OldPack {
    file: File,
    rows: HashMap<String, ([u8; 32], u64, u32)>,
}

fn read_u32(reader: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn write_record(pack: &mut impl Write, path: &str, sha: &[u8; 32], body: &[u8]) -> io::Result<()> {
    pack.write_all(&(path.len() as u32).to_le_bytes())?;
    pack.write_all(path.as_bytes())?;
    pack.write_all(sha)?;
    pack.write_all(&(body.len() as u32).to_le_bytes())?;
    pack.write_all(body)
}

fn pack_header() -> Vec<u8> {
    let mut header = PACK_MAGIC.to_vec();
    header.extend_from_slice(&FACTS_VERSION.to_le_bytes());
    header
}

/// An unreadable or older pack is an empty cache, never an error.
fn open_old_pack(path: &Path) -> Option<OldPack> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file.try_clone().ok()?);
    let mut header = vec![0u8; 12];
    reader.read_exact(&mut header).ok()?;
    if header != pack_header() {
        return None;
    }
    let total = file.metadata().ok()?.len();
    let mut rows = HashMap::new();
    let mut at = 12u64;
    while at < total {
        let path_length = read_u32(&mut reader).ok()?;
        let mut name = vec![0u8; path_length as usize];
        reader.read_exact(&mut name).ok()?;
        let mut sha = [0u8; 32];
        reader.read_exact(&mut sha).ok()?;
        let length = read_u32(&mut reader).ok()?;
        let body = at + 4 + u64::from(path_length) + 32 + 4;
        reader.seek_relative(i64::from(length)).ok()?;
        rows.insert(String::from_utf8(name).ok()?, (sha, body, length));
        at = body + u64::from(length);
    }
    Some(OldPack { file, rows })
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> io::Result<Self> {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!("rust-graph-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&dir)?;
        Ok(Scratch { dir })
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct PackReader {
    file: File,
    entries: Vec<Entry>,
    imports: Cached<Vec<Import>>,
    classes: Cached<Vec<(String, Vec<String>)>>,
}

impl PackReader {
    fn facts(&mut self, id: u32) -> io::Result<Facts> {
        let entry = &self.entries[id as usize];
        let mut body = vec![0u8; entry.length as usize];
        self.file.seek(SeekFrom::Start(entry.offset))?;
        self.file.read_exact(&mut body)?;
        Facts::decode(&body).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("bad record for {}", entry.path)))
    }
}

impl ImportSource for PackReader {
    fn imports_of(&mut self, file: u32) -> Rc<Vec<Import>> {
        if let Some(found) = self.imports.entries.get(&file) {
            return found.clone();
        }
        let loaded = self.facts(file).map(|facts| facts.imports).unwrap_or_default();
        self.imports.get_or(file, || loaded)
    }

    fn bases_of(&mut self, file: u32, class: &str) -> Vec<String> {
        let rows = match self.classes.entries.get(&file) {
            Some(found) => found.clone(),
            None => {
                let symbols = self.facts(file).map(|facts| facts.symbols).unwrap_or_default();
                let rows = symbols.into_iter().filter(|s| s.kind == KIND_CLASS).map(|s| (s.name, s.bases)).collect();
                self.classes.get_or(file, || rows)
            }
        };
        rows.iter().find(|row| row.0 == class).map_or_else(Vec::new, |row| row.1.clone())
    }
}

/// One section of the document, spooled so memory stays flat.
struct Section {
    path: PathBuf,
    writer: BufWriter<File>,
    rows: u64,
}

impl Section {
    fn new(path: PathBuf) -> io::Result<Self> {
        let writer = BufWriter::with_capacity(64 * 1024, File::create(&path)?);
        Ok(Section { path, writer, rows: 0 })
    }

    fn row(&mut self, text: &str) -> io::Result<()> {
        if self.rows > 0 {
            self.writer.write_all(b",\n")?;
        }
        self.rows += 1;
        self.writer.write_all(text.as_bytes())
    }

    fn drain_into(mut self, out: &mut impl Write) -> io::Result<()> {
        self.writer.flush()?;
        io::copy(&mut File::open(&self.path)?, out)?;
        Ok(())
    }
}

#[derive(Default)]
struct Stats {
    languages: BTreeMap<String, (u64, u64)>,
    extracted: u64,
    inferred: u64,
    external: u64,
    unknown: u64,
    called: HashMap<String, u64>,
    ambiguous: HashMap<String, u64>,
    strategies: BTreeMap<&'static str, u64>,
}

fn symbol_row(path: &str, tag: &str, symbol: &Symbol) -> String {
    let kind = match symbol.kind {
        KIND_MODULE => "module",
        KIND_CLASS => "class",
        _ => "function",
    };
    let mut row = String::with_capacity(160);
    row.push_str("{\"id\":");
    push_string(&mut row, &format!("{path}:{}", symbol.key));
    row.push_str(",\"name\":");
    push_string(&mut row, &symbol.name);
    row.push_str(",\"owner\":");
    push_string(&mut row, &symbol.owner);
    row.push_str(",\"file\":");
    push_string(&mut row, path);
    row.push_str(&format!(",\"line\":{},\"end_line\":{},\"kind\":\"{kind}\",\"lang\":\"{tag}\",\"params\":", symbol.line, symbol.end_line));
    push_list(&mut row, &symbol.params);
    if !symbol.bases.is_empty() {
        row.push_str(",\"bases\":");
        push_list(&mut row, &symbol.bases);
    }
    row.push('}');
    row
}

fn top(map: HashMap<String, u64>) -> Vec<(String, u64)> {
    let mut rows: Vec<(String, u64)> = map.into_iter().collect();
    rows.sort_unstable_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    rows.truncate(TOP_LIST);
    rows
}

fn report(totals: &Totals, stats: Stats, files: usize, symbols: u64) -> String {
    let mut text = String::from("# rust-graph report\n\n");
    text.push_str(&format!(
        "{files} files, {symbols} symbols, {} call edges ({} EXTRACTED, {} INFERRED).\n",
        stats.extracted + stats.inferred,
        stats.extracted,
        stats.inferred
    ));
    text.push_str(&format!(
        "{} calls left the tree through an import, {} named nothing the tree defines, {} were ambiguous.\n",
        stats.external,
        stats.unknown,
        stats.ambiguous.values().sum::<u64>()
    ));
    if totals.too_large > 0 {
        text.push_str(&format!("{} files were over the size limit and not read.\n", totals.too_large));
    }
    if totals.truncated {
        text.push_str("The file limit was reached: this graph does not cover the whole tree.\n");
    }
    text.push_str("\n## Languages\n\n| language | files | symbols |\n| --- | ---: | ---: |\n");
    for (tag, (count, defined)) in &stats.languages {
        text.push_str(&format!("| {tag} | {count} | {defined} |\n"));
    }
    text.push_str("\n## How calls were resolved\n\n| strategy | edges |\n| --- | ---: |\n");
    for (strategy, count) in &stats.strategies {
        text.push_str(&format!("| {strategy} | {count} |\n"));
    }
    text.push_str("\n## Most called\n\n");
    for (name, count) in top(stats.called) {
        text.push_str(&format!("- {count} × `{name}`\n"));
    }
    let ambiguous = top(stats.ambiguous);
    if !ambiguous.is_empty() {
        text.push_str("\n## Most ambiguous names\n\n");
        for (name, count) in ambiguous {
            text.push_str(&format!("- {count} × `{name}`\n"));
        }
    }
    text
}

fn scope_of(tag: &str) -> Scope {
    let row = langs::LANGUAGES.iter().find(|language| language.tag == tag);
    Scope {
        globals: tag == "M" || row.is_some_and(|language| language.global_names),
        members: row.is_some_and(|language| language.member_calls),
    }
}

fn parse_pass(
    options: &Options,
    paths: &[String],
    old: &mut Option<OldPack>,
    pack_path: &Path,
    index: &mut Index,
    totals: &mut Totals,
) -> io::Result<Vec<Entry>> {
    let mut pack = BufWriter::with_capacity(256 * 1024, File::create(pack_path)?);
    pack.write_all(&pack_header())?;
    let mut at = 12u64;
    let mut entries = Vec::with_capacity(paths.len());
    let mut content = Vec::new();
    for path in paths {
        let full = options.root.join(path);
        let Ok(metadata) = fs::metadata(&full) else { continue };
        if metadata.len() > options.max_file_bytes {
            totals.too_large += 1;
            continue;
        }
        content.clear();
        if File::open(&full).and_then(|mut file| file.read_to_end(&mut content)).is_err() {
            continue;
        }
        let sha = sha256::digest(&content);
        let reused = old.as_mut().and_then(|pack| {
            let (old_sha, offset, length) = *pack.rows.get(path)?;
            if old_sha != sha {
                return None;
            }
            let mut body = vec![0u8; length as usize];
            pack.file.seek(SeekFrom::Start(offset)).ok()?;
            pack.file.read_exact(&mut body).ok()?;
            Some(body)
        });
        let (body, facts) = match reused.and_then(|body| Facts::decode(&body).map(|facts| (body, facts))) {
            Some(pair) => {
                totals.reused += 1;
                pair
            }
            None => {
                let Some(language) = language_of(path, &content) else { continue };
                let facts = parser::parse(&content, language);
                totals.parsed += 1;
                (facts.encode(), facts)
            }
        };
        write_record(&mut pack, path, &sha, &body)?;
        let offset = at + 4 + path.len() as u64 + 32 + 4;
        at = offset + body.len() as u64;
        index.add_file(path, &facts);
        totals.bytes += content.len() as u64;
        entries.push(Entry { path: path.clone(), sha, bytes: content.len() as u64, offset, length: body.len() as u32 });
    }
    pack.flush()?;
    Ok(entries)
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

struct Sections {
    sources: Section,
    symbols: Section,
    edges: Section,
    imports: Section,
    unresolved: Section,
    flows: Section,
}

fn resolve_file(
    id: u32,
    facts: &Facts,
    path: &str,
    reader: &mut PackReader,
    index: &Index,
    out: &mut Sections,
    stats: &mut Stats,
) -> io::Result<()> {
    let scope = scope_of(&facts.tag);
    let mut resolver = Resolver::new(index, reader);
    for import in &facts.imports {
        let mut row = String::from("{\"file\":");
        push_string(&mut row, path);
        row.push_str(&format!(",\"line\":{},\"module\":", import.line));
        push_string(&mut row, &import.module);
        row.push_str(",\"name\":");
        push_string(&mut row, &import.name);
        row.push_str(",\"alias\":");
        push_string(&mut row, &import.alias);
        row.push_str(&format!(",\"star\":{},\"target\":", import.kind == IMPORT_STAR));
        let target = import_target(&resolver, import, id);
        push_optional(&mut row, target.as_deref());
        row.push_str(",\"tag\":\"EXTRACTED\"}");
        out.imports.row(&row)?;
    }
    for symbol in &facts.symbols {
        let caller = format!("{path}:{}", symbol.key);
        if symbol.kind != KIND_MODULE {
            out.symbols.row(&symbol_row(path, &facts.tag, symbol))?;
        } else if !symbol.events.is_empty() {
            out.symbols.row(&symbol_row(path, &facts.tag, symbol))?;
        }
        let mut calls = String::from("[");
        for (position, call) in symbol.calls.iter().enumerate() {
            if position > 0 {
                calls.push(',');
            }
            calls.push('[');
            push_string(&mut calls, &call.path);
            calls.push_str(&format!(",{},", call.line));
            match resolver.call(&call.path, id, &symbol.owner, facts, scope) {
                Outcome::Bound(bound) => {
                    let target = index.id(&bound.row);
                    let mut edge = String::from("{\"caller\":");
                    push_string(&mut edge, &caller);
                    edge.push_str(",\"callee\":");
                    push_string(&mut edge, &target);
                    edge.push_str(&format!(
                        ",\"line\":{},\"kind\":\"call\",\"tag\":\"{}\",\"strategy\":\"{}\"}}",
                        call.line,
                        bound.tag.text(),
                        bound.strategy
                    ));
                    out.edges.row(&edge)?;
                    match bound.tag {
                        Tag::Extracted => stats.extracted += 1,
                        Tag::Inferred => stats.inferred += 1,
                    }
                    *stats.strategies.entry(bound.strategy).or_default() += 1;
                    *stats.called.entry(target.clone()).or_default() += 1;
                    push_string(&mut calls, &target);
                    calls.push_str(&format!(",null,{}", u8::from(bound.constructor)));
                }
                Outcome::External(canonical) => {
                    stats.external += 1;
                    calls.push_str("null,");
                    push_string(&mut calls, &canonical);
                    calls.push_str(",0");
                }
                Outcome::Ambiguous => {
                    let mut row = String::from("{\"caller\":");
                    push_string(&mut row, &caller);
                    row.push_str(",\"name\":");
                    push_string(&mut row, &call.path);
                    row.push_str(&format!(",\"line\":{},\"reason\":\"ambiguous\"}}", call.line));
                    out.unresolved.row(&row)?;
                    *stats.ambiguous.entry(call.path.clone()).or_default() += 1;
                    calls.push_str("null,null,0");
                }
                Outcome::Unknown => {
                    stats.unknown += 1;
                    calls.push_str("null,null,0");
                }
            }
            calls.push(']');
        }
        calls.push(']');
        if !symbol.events.is_empty() {
            let mut row = String::with_capacity(symbol.events.len() + calls.len() + 64);
            row.push_str("{\"symbol\":");
            push_string(&mut row, &caller);
            row.push_str(",\"events\":[");
            row.push_str(&symbol.events);
            row.push_str("],\"calls\":");
            row.push_str(&calls);
            row.push('}');
            out.flows.row(&row)?;
        }
    }
    Ok(())
}

/// The kernel's high-water mark for this process; 0 where there is no procfs.
fn peak_rss_kb() -> u64 {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let line = status.lines().find(|line| line.starts_with("VmHWM:")).unwrap_or("");
    line.split_whitespace().nth(1).and_then(|value| value.parse().ok()).unwrap_or(0)
}

pub fn build(options: &Options) -> io::Result<()> {
    let started = Instant::now();
    if !options.root.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("{} is not a directory", options.root.display())));
    }
    let mut paths = Vec::new();
    walk(&options.root, "", &mut paths)?;
    let mut totals = Totals::default();
    if paths.len() > options.max_files {
        paths.truncate(options.max_files);
        totals.truncated = true;
    }

    let scratch = Scratch::new()?;
    let cache_dir = (!options.no_cache).then(|| options.cache.clone().unwrap_or_else(|| options.root.join(".rust-graph")));
    let old_path = cache_dir.as_ref().map(|dir| dir.join("facts.pack"));
    let mut old = old_path.as_deref().and_then(open_old_pack);
    let pack_path = scratch.file("facts.pack");
    let mut index = Index::default();
    let entries = parse_pass(options, &paths, &mut old, &pack_path, &mut index, &mut totals)?;
    drop(old);
    drop(paths);
    index.seal();

    let mut reader = PackReader {
        file: File::open(&pack_path)?,
        entries,
        imports: Cached::new(256),
        classes: Cached::new(256),
    };
    let mut sections = Sections {
        sources: Section::new(scratch.file("sources"))?,
        symbols: Section::new(scratch.file("symbols"))?,
        edges: Section::new(scratch.file("edges"))?,
        imports: Section::new(scratch.file("imports"))?,
        unresolved: Section::new(scratch.file("unresolved"))?,
        flows: Section::new(scratch.file("flows"))?,
    };
    let mut stats = Stats::default();
    let mut symbol_count = 0u64;
    let total_files = reader.entries.len();
    for id in 0..total_files as u32 {
        let facts = reader.facts(id)?;
        let entry = &reader.entries[id as usize];
        let path = entry.path.clone();
        let mut row = String::from("{\"path\":");
        push_string(&mut row, &path);
        row.push_str(&format!(
            ",\"lang\":\"{}\",\"sha256\":\"{}\",\"bytes\":{},\"lines\":{}}}",
            facts.tag,
            hex(&entry.sha),
            entry.bytes,
            facts.lines
        ));
        sections.sources.row(&row)?;
        let defined = facts.symbols.iter().filter(|symbol| symbol.kind != KIND_MODULE).count() as u64;
        symbol_count += defined;
        let language = stats.languages.entry(facts.tag.clone()).or_default();
        language.0 += 1;
        language.1 += defined;
        resolve_file(id, &facts, &path, &mut reader, &index, &mut sections, &mut stats)?;
    }

    let render = |out: &mut dyn Write, sections: Sections| -> io::Result<()> {
        write!(
            out,
            "{{\"version\":1,\"tool\":\"rust-graph\",\"files\":{},\"bytes\":{},\"truncated\":{},\"too_large\":{}",
            total_files, totals.bytes, totals.truncated, totals.too_large
        )?;
        let parts = [
            ("sources", sections.sources),
            ("symbols", sections.symbols),
            ("edges", sections.edges),
            ("imports", sections.imports),
            ("unresolved", sections.unresolved),
            ("flows", sections.flows),
        ];
        for (name, section) in parts {
            write!(out, ",\n\"{name}\":[\n")?;
            let mut boxed: Box<&mut dyn Write> = Box::new(out);
            section.drain_into(&mut boxed)?;
            out.write_all(b"\n]")?;
        }
        out.write_all(b"}\n")
    };

    let target_dir = options.out.clone().or_else(|| (!options.stdout).then(|| options.root.join(".rust-graph")));
    if options.stdout {
        let stdout = io::stdout();
        let mut out = BufWriter::with_capacity(256 * 1024, stdout.lock());
        render(&mut out, sections)?;
        out.flush()?;
    } else if let Some(dir) = &target_dir {
        fs::create_dir_all(dir)?;
        let temporary = dir.join("graph.json.tmp");
        let mut out = BufWriter::with_capacity(256 * 1024, File::create(&temporary)?);
        render(&mut out, sections)?;
        out.flush()?;
        drop(out);
        fs::rename(&temporary, dir.join("graph.json"))?;
    }
    let text = report(&totals, stats, total_files, symbol_count);
    if let Some(dir) = target_dir.as_ref().filter(|_| !options.stdout) {
        fs::write(dir.join("report.md"), &text)?;
    }
    if let Some(dir) = cache_dir {
        fs::create_dir_all(&dir)?;
        let temporary = dir.join("facts.pack.tmp");
        fs::copy(&pack_path, &temporary)?;
        fs::rename(&temporary, dir.join("facts.pack"))?;
    }
    eprintln!(
        "rust-graph: {} files ({} parsed, {} cached), {symbol_count} symbols, {} ms, peak {} KB",
        total_files,
        totals.parsed,
        totals.reused,
        started.elapsed().as_millis(),
        peak_rss_kb()
    );
    Ok(())
}
