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
    pub max_file_bytes: u64, #[cfg(all(feature = "deep", feature = "parser"))] pub path: PathMode,
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
            max_file_bytes: 2 * 1024 * 1024, #[cfg(all(feature = "deep", feature = "parser"))] path: PathMode::Auto,
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
    #[cfg(not(all(feature = "deep", feature = "parser")))] header.extend_from_slice(&FACTS_VERSION.to_le_bytes()); #[cfg(all(feature = "deep", feature = "parser"))] header.extend_from_slice(&(FACTS_VERSION | (deep_run::bits() << 8)).to_le_bytes());
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
                #[cfg(not(all(feature = "deep", feature = "parser")))] let facts = parser::parse(&content, language); #[cfg(all(feature = "deep", feature = "parser"))] let facts = parse_source(&content, path, language);
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
    let mut paths = Vec::new(); #[cfg(all(feature = "deep", feature = "parser"))] deep_run::set(options.path);
    walk(&options.root, "", &mut paths)?;
    let mut totals = Totals::default();
    if paths.len() > options.max_files {
        paths.truncate(options.max_files);
        totals.truncated = true;
    }

    let scratch = Scratch::new()?;
    let cache_dir = (!options.no_cache).then(|| options.cache.clone().unwrap_or_else(|| options.root.join(".rust-graph")));
    let old_path = cache_dir.as_ref().map(|dir| dir.join("facts.pack"));
    let mut old = old_path.as_deref().and_then(open_old_pack); #[cfg(all(feature = "deep", feature = "parser"))] let old_meta = cache_dir.as_ref().and_then(|dir| deep_run::load(dir));
    let pack_path = scratch.file("facts.pack");
    let mut index = Index::default(); #[cfg(all(feature = "deep", feature = "parser"))] let mut parser_state = deep_run::State::default();
    let entries = parse_pass(options, &paths, &mut old, &pack_path, &mut index, &mut totals)?; #[cfg(all(feature = "deep", feature = "parser"))] { parser_state.absorb(); parser_state.merge(old_meta); if deep_run::compare_mode() { deep_run::print_compare()?; return Ok(()); } }
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
        )); #[cfg(all(feature = "deep", feature = "parser"))] parser_state.note(&mut row, &path);
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
        )?; #[cfg(all(feature = "deep", feature = "parser"))] parser_state.heading(out)?;
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
    let text = report(&totals, stats, total_files, symbol_count); #[cfg(all(feature = "deep", feature = "parser"))] let text = parser_state.report_line(&text);
    if let Some(dir) = target_dir.as_ref().filter(|_| !options.stdout) {
        fs::write(dir.join("report.md"), &text)?;
    }
    if let Some(dir) = cache_dir {
        fs::create_dir_all(&dir)?;
        let temporary = dir.join("facts.pack.tmp");
        fs::copy(&pack_path, &temporary)?;
        fs::rename(&temporary, dir.join("facts.pack"))?; #[cfg(all(feature = "deep", feature = "parser"))] parser_state.save(&dir)?;
    }
    eprintln!(
        "rust-graph: {} files ({} parsed, {} cached), {symbol_count} symbols, {} ms, peak {} KB",
        total_files,
        totals.parsed,
        totals.reused,
        started.elapsed().as_millis(),
        peak_rss_kb()
    ); #[cfg(all(feature = "deep", feature = "parser"))] eprintln!("{}", parser_state.stderr_line());
    Ok(())
}

// ---------------------------------------------------------------------------
// The second parser: features `deep` and `parser`.
//
// Everything in this section compiles only when both features are on, and it
// sits after the last line the default build already had. The placement is
// measured, not cosmetic: a panic site stores its file and line, so adding a
// line anywhere above would move bytes in the default binary even with the
// code `cfg`'d out. The hooks in the pipeline above are therefore one line
// each — a `cfg`-chosen call, never a new block — and this section is where
// the work they name lives. `crates/deep-bench/default-build.sha256` records
// what the default binary hashes to.
// ---------------------------------------------------------------------------

/// Which parser a build uses: the one-pass lexer, the CST, or — the default —
/// the CST where this build has one for the language and the lexer elsewhere.
#[cfg(all(feature = "deep", feature = "parser"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PathMode {
    #[default]
    Auto,
    Deep,
    Lexer,
    /// Parse both ways, print the per-file difference, choose nothing. An
    /// audit, not a build: no graph is written and no cache is kept.
    Compare,
}

#[cfg(all(feature = "deep", feature = "parser"))]
impl PathMode {
    fn description(self) -> &'static str {
        match self {
            PathMode::Auto => "auto",
            PathMode::Deep => "deep",
            PathMode::Lexer => "lexer",
            PathMode::Compare => "compare",
        }
    }
}

/// The bridge, re-exported under the name `src/deep_facts.rs` already uses.
///
/// The bridge is a module of the pipeline rather than of the crate root so that
/// `src/main.rs` never has to name it: the crate-root glob in `main.rs` is what
/// gives `crate::deep_integration` its meaning, and it exists only under the
/// feature gate.
#[cfg(all(feature = "deep", feature = "parser"))]
#[path = "deep_integration.rs"]
pub(crate) mod bridge;

#[cfg(all(feature = "deep", feature = "parser"))]
pub(crate) mod gate {
    pub(crate) use super::bridge as deep_integration;
}

#[cfg(all(feature = "deep", feature = "parser"))]
#[path = "deep_facts.rs"]
mod deep_facts;

/// The parser this run is using, and the per-file record of what each file
/// took.
///
/// `pack_header` and `parse_source` both need the mode and neither has the
/// `Options` in hand, so it lives here: `build` sets it once, and a build is
/// one thread. The per-file record is written beside the pack, because the pack
/// holds facts and a pak of facts cannot say which parser wrote them.
#[cfg(all(feature = "deep", feature = "parser"))]
mod deep_run {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io::{self, Write};
    use std::path::Path;
    use std::sync::atomic::{AtomicU8, Ordering};

    use super::PathMode;

    static MODE: AtomicU8 = AtomicU8::new(0);

    pub(super) fn set(mode: PathMode) {
        MODE.store(mode as u8, Ordering::Relaxed);
    }

    pub(super) fn get() -> PathMode {
        match MODE.load(Ordering::Relaxed) {
            1 => PathMode::Deep,
            2 => PathMode::Lexer,
            3 => PathMode::Compare,
            _ => PathMode::Auto,
        }
    }

    /// The mode's slot in the pack header's version word: a pack written by one
    /// parser is never served to another, which is what keeps a lexer-built
    /// cache from silently standing in for a requested CST parse.
    pub(super) fn bits() -> u32 {
        match get() {
            PathMode::Auto => 1,
            PathMode::Deep => 2,
            PathMode::Lexer => 3,
            // A comparison run is never a cache producer or consumer: its own
            // word means the pack and the provenance file it meets are both
            // void, so an audit always reads the tree as it stands.
            PathMode::Compare => 4,
        }
    }

    /// What one file's facts went through.
    #[derive(Clone, Default)]
    pub(super) struct Taken {
        /// The emitted record is the CST's.
        pub(super) deep: bool,
        /// Both records existed and were weighed against each other.
        pub(super) compared: bool,
        /// The CST's own count of `ERROR` nodes: recovery, not a fallback.
        pub(super) errors: u32,
        /// Written only when the deep path was expected and not taken.
        pub(super) fallback: Option<String>,
        /// Why the emitted record won: `"tie"`, `"lexer"` (the lexer had more
        /// facts) or `""` (the CST had more, or there was nothing to compare).
        pub(super) why: &'static str,
    }

    thread_local! {
        // Pass one decides and pass two renders; a `String` and a flag per
        // parsed file is the whole cost.
        static RECORDS: RefCell<Vec<(String, Taken)>> = const { RefCell::new(Vec::new()) };
        // Only `--compare` writes here: one row per file with both weights.
        static COMPARED: RefCell<Vec<Compared>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn record(path: &str, taken: Taken) {
        RECORDS.with(|records| records.borrow_mut().push((path.to_string(), taken)));
    }

    /// One file's audit row: its weight on each path, and the CST's errors.
    pub(super) struct Compared {
        path: String,
        /// `None` when this build has no CST for the language, or the CST
        /// returned no tree: the lexer stood alone and there is nothing to
        /// compare.
        deep: Option<[u64; 3]>,
        lexer: [u64; 3],
        errors: u32,
    }

    pub(super) fn record_compare(path: &str, deep: Option<([u64; 3], u32)>, lexer: [u64; 3]) {
        let row = Compared { path: path.to_string(), deep: deep.map(|pair| pair.0), lexer, errors: deep.map_or(0, |pair| pair.1) };
        COMPARED.with(|rows| rows.borrow_mut().push(row));
    }

    pub(super) fn compare_mode() -> bool {
        get() == PathMode::Compare
    }

    /// Print the audit: one line per file with both paths' counts and the
    /// difference, then the totals. No winner is picked and nothing is built.
    pub(super) fn print_compare() -> io::Result<()> {
        let mut rows = COMPARED.with(|rows| std::mem::take(&mut *rows.borrow_mut()));
        rows.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        let stdout = io::stdout();
        let mut out = io::BufWriter::new(stdout.lock());
        let mut totals = [0u64; 6];
        let mut compared = 0usize;
        for row in &rows {
            match row.deep {
                Some(deep) => {
                    compared += 1;
                    for side in 0..3 {
                        totals[side * 2] += deep[side];
                        totals[side * 2 + 1] += row.lexer[side];
                    }
                    write!(
                        out,
                        "{}\tsymbols deep={} lexer={} ({:+})\tcalls deep={} lexer={} ({:+})\timports deep={} lexer={} ({:+})",
                        row.path,
                        deep[0],
                        row.lexer[0],
                        deep[0] as i64 - row.lexer[0] as i64,
                        deep[1],
                        row.lexer[1],
                        deep[1] as i64 - row.lexer[1] as i64,
                        deep[2],
                        row.lexer[2],
                        deep[2] as i64 - row.lexer[2] as i64
                    )?;
                    if row.errors > 0 {
                        write!(out, "\tcst errors={}", row.errors)?;
                    }
                    writeln!(out)?;
                }
                None => {
                    for side in 0..3 {
                        totals[side * 2 + 1] += row.lexer[side];
                    }
                    writeln!(
                        out,
                        "{}\tno CST record\tsymbols lexer={}\tcalls lexer={}\timports lexer={}",
                        row.path, row.lexer[0], row.lexer[1], row.lexer[2]
                    )?;
                }
            }
        }
        writeln!(out, "compare: {} files, {compared} with a CST record, {} lexer-only", rows.len(), rows.len() - compared)?;
        writeln!(
            out,
            "compare totals: symbols deep={} lexer={} ({:+}), calls deep={} lexer={} ({:+}), imports deep={} lexer={} ({:+})",
            totals[0],
            totals[1],
            totals[0] as i64 - totals[1] as i64,
            totals[2],
            totals[3],
            totals[2] as i64 - totals[3] as i64,
            totals[4],
            totals[5],
            totals[4] as i64 - totals[5] as i64
        )?;
        out.flush()
    }

    #[derive(Default)]
    pub(super) struct State {
        records: HashMap<String, Taken>,
        /// Files whose emitted record is the CST's. This is `chosen_deep`.
        deep: u64,
        /// Files whose emitted record is the lexer's. This is `chosen_lexer`.
        lexer: u64,
        /// Files both paths ran on: `paths.deep`.
        compared: u64,
        /// Files only the lexer could take: `paths.lexer`.
        lexer_only: u64,
        fallbacks: u64,
        reused: u64,
    }

    impl State {
        /// Move pass one's records in, once pass one has finished.
        pub(super) fn absorb(&mut self) {
            self.records = RECORDS.with(|records| std::mem::take(&mut *records.borrow_mut())).into_iter().collect();
        }

        /// Fill in the files the pack supplied, from the provenance file that
        /// was written with it. Without this a warm build could not say which
        /// parser its facts came from and its output would differ from a cold
        /// one for no reason a reader could see.
        pub(super) fn merge(&mut self, old: Option<HashMap<String, Taken>>) {
            if let Some(old) = old {
                for (path, taken) in old {
                    self.records.entry(path).or_insert(taken);
                }
            }
        }

        /// The provenance for every file in the pack, for the next build. The
        /// header carries the format number, so a file from an older writer is
        /// void rather than half-read.
        pub(super) fn save(&self, dir: &Path) -> io::Result<()> {
            let mut text = format!("rust-graph facts.meta 2 {}\n", bits());
            let mut rows: Vec<(&String, &Taken)> = self.records.iter().collect();
            rows.sort_unstable_by(|left, right| left.0.cmp(right.0));
            for (path, taken) in rows {
                let parser = match (taken.deep, taken.fallback.is_some()) {
                    (true, _) => "deep",
                    (false, true) => "lexer(fallback)",
                    (false, false) => "lexer",
                };
                text.push_str(&format!(
                    "{} {} {} {} {} {}\n",
                    escape(path),
                    parser,
                    taken.errors,
                    u8::from(taken.compared),
                    taken.why,
                    escape(taken.fallback.as_deref().unwrap_or(""))
                ));
            }
            let temporary = dir.join("facts.meta.tmp");
            std::fs::write(&temporary, text)?;
            std::fs::rename(&temporary, dir.join("facts.meta"))
        }

        /// Write one file's provenance into its `sources` row.
        pub(super) fn note(&mut self, row: &mut String, path: &str) {
            let taken = self.records.get(path);
            let parsed_by = match taken {
                Some(taken) if taken.deep => {
                    self.deep += 1;
                    "deep"
                }
                Some(_) => {
                    self.lexer += 1;
                    "lexer"
                }
                // Only when a cache predates the provenance file: the facts are
                // the pack's and which parser wrote them is not recorded.
                None => {
                    self.reused += 1;
                    "cache"
                }
            };
            // The row was built closed; open it again for the parser fields.
            row.truncate(row.len() - 1);
            if let Some(taken) = taken {
                if taken.compared {
                    self.compared += 1;
                } else {
                    self.lexer_only += 1;
                }
                let choice = match (taken.deep, taken.why) {
                    (true, "tie") => Some("deep:tie"),
                    (true, _) => Some("deep:more"),
                    (false, _) if taken.fallback.is_some() => Some("lexer:fallback"),
                    (false, "lexer") => Some("lexer:more"),
                    (false, _) => None,
                };
                if let Some(choice) = choice {
                    row.push_str(",\"path_choice\":\"");
                    row.push_str(choice);
                    row.push('"');
                }
            }
            row.push_str(",\"parsed_by\":\"");
            row.push_str(parsed_by);
            row.push('"');
            if let Some(taken) = taken {
                if taken.errors > 0 {
                    row.push_str(&format!(",\"deep_errors\":{}", taken.errors));
                }
                if let Some(reason) = taken.fallback.as_deref() {
                    self.fallbacks += 1;
                    row.push_str(",\"deep_fallback\":");
                    super::push_string(row, reason);
                }
            }
            row.push('}');
        }

        /// One object a reader can count without opening `sources`.
        pub(super) fn heading(&self, out: &mut dyn Write) -> io::Result<()> {
            write!(
                out,
                ",\"deep\":{{\"mode\":\"{}\",\"deep\":{},\"lexer\":{},\"fallbacks\":{}}}",
                get().description(),
                self.deep,
                self.lexer,
                self.fallbacks
            )?;
            // Which path each file's facts came from, in one object. `deep` and
            // `lexer` here are candidates (a CST record existed / the lexer
            // stood alone); `chosen_*` is the outcome per file.
            write!(
                out,
                ",\"paths\":{{\"deep\":{},\"lexer\":{},\"chosen_deep\":{},\"chosen_lexer\":{}}}",
                self.compared, self.lexer_only, self.deep, self.lexer
            )
        }

        pub(super) fn report_line(&self, text: &str) -> String {
            format!(
                "{text}\nParsed by: {} files through the CST, {} through the lexer, {} requested CST parses fell back.\nPaths: {{\"deep\": {}, \"lexer\": {}, \"chosen_deep\": {}, \"chosen_lexer\": {}}} (files with a CST record, files the lexer alone could take; which record was emitted per file).\n",
                self.deep, self.lexer, self.fallbacks, self.compared, self.lexer_only, self.deep, self.lexer
            )
        }

        pub(super) fn stderr_line(&self) -> String {
            format!(
                "rust-graph: parser {}, {} deep, {} lexer, {} fallbacks, {} cached",
                get().description(),
                self.deep,
                self.lexer,
                self.fallbacks,
                self.reused
            )
        }
    }

    /// Load the provenance written beside a previous pack, or `None` when there
    /// is none or it was written by another parser. The pack header already
    /// rejects a foreign pack; this keeps the two in step.
    pub(super) fn load(dir: &Path) -> Option<HashMap<String, Taken>> {
        let text = std::fs::read_to_string(dir.join("facts.meta")).ok()?;
        let mut lines = text.lines();
        if lines.next()? != format!("rust-graph facts.meta 2 {}", bits()) {
            return None;
        }
        let mut out = HashMap::new();
        for line in lines {
            let mut fields = line.split(' ');
            let path = unescape(fields.next()?);
            let parser = fields.next()?;
            let errors: u32 = fields.next()?.parse().ok()?;
            let compared = fields.next()? == "1";
            let why = match fields.next()? {
                "tie" => "tie",
                "lexer" => "lexer",
                _ => "",
            };
            let reason = unescape(fields.next().unwrap_or(""));
            out.insert(
                path,
                Taken { deep: parser == "deep", compared, errors, fallback: (!reason.is_empty()).then_some(reason), why },
            );
        }
        Some(out)
    }

    /// A path may hold a space; a newline or a tab would break the line format.
    fn escape(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for ch in text.chars() {
            match ch {
                '%' => out.push_str("%25"),
                ' ' => out.push_str("%20"),
                '\n' => out.push_str("%0A"),
                '\t' => out.push_str("%09"),
                _ => out.push(ch),
            }
        }
        out
    }

    fn unescape(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let bytes = text.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'%' && index + 2 < bytes.len() {
                let hex = &text[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte as char);
                    index += 3;
                    continue;
                }
            }
            out.push(bytes[index] as char);
            index += 1;
        }
        out
    }
}

/// The three counts two records are weighed by, in order: symbols, then calls,
/// then imports. A record is "better" when the first count that differs is
/// larger, so a CST that finds one more symbol wins even if the lexer found
/// three more calls — a missing definition is the costlier loss.
#[cfg(all(feature = "deep", feature = "parser"))]
fn weight(facts: &Facts) -> [u64; 3] {
    let calls: u64 = facts.symbols.iter().map(|symbol| symbol.calls.len() as u64).sum();
    [facts.symbols.len() as u64, calls, facts.imports.len() as u64]
}

/// Whether the CST record lost anything the lexer's found.
///
/// The deep walk is the better reader where its tables cover a construct, but
/// where a grammar or a table has not reached one it can silently see less than
/// the one-pass lexer — `from m import *`, `a, b = f(x), 1`, `f"{x}"` are three
/// the differential fixtures do not carry. `auto` therefore compares the two
/// records and keeps the lexer's when the CST's is missing something: fewer
/// imports, a named symbol the lexer had, a call, or a flow event. A loss is
/// recorded as a fallback rather than hidden, and `--deep` skips the comparison
/// because the user asked for the CST.
#[cfg(all(feature = "deep", feature = "parser"))]
fn loses(lexer: &Facts, deep: &Facts) -> bool {
    for import in &lexer.imports {
        let same = deep.imports.iter().any(|row| {
            row.module == import.module && row.name == import.name && row.alias == import.alias && row.kind == import.kind
        });
        if !same {
            return true;
        }
    }
    for symbol in lexer.symbols.iter().filter(|symbol| symbol.kind != KIND_MODULE) {
        let same = deep
            .symbols
            .iter()
            .any(|row| row.name == symbol.name && row.kind == symbol.kind);
        if !same {
            return true;
        }
    }
    let name_of = |path: &str| path.rsplit('.').next().unwrap_or(path).to_string();
    for symbol in &lexer.symbols {
        for call in &symbol.calls {
            let name = name_of(&call.path);
            let near = |line: u32| (line as i64 - call.line as i64).abs() <= 1;
            let same = deep
                .symbols
                .iter()
                .flat_map(|row| row.calls.iter())
                .any(|row| name_of(&row.path) == name && near(row.line));
            if !same {
                return true;
            }
        }
    }
    let events = |facts: &Facts, marker: &str| -> usize {
        facts.symbols.iter().map(|symbol| symbol.events.matches(marker).count()).sum()
    };
    for marker in ["[\"a\",", "[\"r\",", "[\"x\","] {
        if events(deep, marker) < events(lexer, marker) {
            return true;
        }
    }
    // A value the lexer read and the CST wrote as a constant on the same line.
    // `f"{x}"` is the live case: the grammar keeps the whole string as one
    // token, so the CST has no field to read and says `0` where the lexer
    // follows the interpolation. The event count above cannot see it because
    // both paths still write the event, only with a different payload.
    for symbol in &lexer.symbols {
        let paired = if symbol.kind == KIND_MODULE {
            deep.symbols.iter().find(|row| row.kind == KIND_MODULE)
        } else {
            deep.symbols.iter().find(|row| row.name == symbol.name && row.kind == symbol.kind)
        };
        let Some(paired) = paired else { continue };
        let deep_values = value_events(&paired.events);
        for (marker, line, constant) in value_events(&symbol.events) {
            if constant {
                continue;
            }
            if deep_values.iter().any(|(deep_marker, deep_line, deep_constant)| {
                *deep_marker == marker && *deep_line == line && *deep_constant
            }) {
                return true;
            }
        }
    }
    false
}

/// The `["r",line,value]` and `["x",line,value]` events in a record, as
/// `(marker, line, is_constant)`. Those two kinds end with their value, so a
/// payload of exactly `0` is the constant form; every other payload is a value
/// the reader of the record can follow.
#[cfg(all(feature = "deep", feature = "parser"))]
fn value_events(events: &str) -> Vec<(u8, u32, bool)> {
    let bytes = events.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at + 6 < bytes.len() {
        if bytes[at] != b'[' || bytes[at + 1] != b'"' || bytes[at + 3] != b'"' || bytes[at + 4] != b',' {
            at += 1;
            continue;
        }
        let marker = bytes[at + 2];
        if !matches!(marker, b'r' | b'x') {
            at += 1;
            continue;
        }
        let mut index = at + 5;
        let mut line = 0u32;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            line = line.saturating_mul(10).saturating_add((bytes[index] - b'0') as u32);
            index += 1;
        }
        if bytes.get(index) != Some(&b',') {
            at += 1;
            continue;
        }
        let payload = index + 1;
        // The event ends at the `]` that closes the `[` this scan started on.
        let mut depth = 0u32;
        let mut end = payload;
        while end < bytes.len() {
            match bytes[end] {
                b'[' => depth += 1,
                b']' if depth == 0 => break,
                b']' => depth -= 1,
                _ => {}
            }
            end += 1;
        }
        let constant = &bytes[payload..end.min(bytes.len())] == b"0";
        out.push((marker, line, constant));
        at = end.max(at + 1);
    }
    out
}

/// The facts for one file, and the record of which parser produced them.
///
/// `--lexer` is the old path unchanged. `--deep` asks for the CST. `auto` runs
/// both where `deep_facts` can read a tree, and then decides:
///
/// 1. if the CST record lost any fact the lexer found (`loses`), the lexer's
///    record is kept and the loss is a counted fallback;
/// 2. otherwise the record with the larger weight wins — symbols, then calls,
///    then imports (`weight`), the first difference deciding;
/// 3. an exact tie goes to the CST, and the row says `"path_choice":"deep:tie"`
///    so a reader can see the coin was not silently flipped.
///
/// `--compare` runs both and records their weights without choosing; the build
/// prints the audit and stops before writing anything.
#[cfg(all(feature = "deep", feature = "parser"))]
fn parse_source(bytes: &[u8], path: &str, language: &'static Language) -> Facts {
    use gate::deep_integration::{self, Lang};

    let mode = deep_run::get();
    if mode == PathMode::Lexer {
        deep_run::record(path, deep_run::Taken::default());
        return parser::parse(bytes, language);
    }
    // `deep_facts::lang_for` is keyed by the lexer tag; TSX shares TS's tag but
    // not its grammar, so the suffix decides between them.
    let lang = match deep_facts::lang_for(language.tag) {
        Some(Lang::TypeScript) if path.ends_with(".tsx") => Some(Lang::Tsx),
        other => other,
    };
    let Some(lang) = lang else {
        // Forcing the CST on a language this build has no table for is a
        // downgrade and says so. Under `auto` the language was never eligible,
        // so the lexer is the choice and not a fallback.
        let fallback = (mode == PathMode::Deep).then(|| format!("no deep parser for {}", language.tag));
        deep_run::record(path, deep_run::Taken { fallback, ..deep_run::Taken::default() });
        let lexer = parser::parse(bytes, language);
        if mode == PathMode::Compare {
            deep_run::record_compare(path, None, weight(&lexer));
        }
        return lexer;
    };
    let lexer = parser::parse(bytes, language);
    let Some(cst) = deep_integration::parse_file(bytes, lang) else {
        let fallback = Some("the deep parser returned no tree".to_string());
        deep_run::record(path, deep_run::Taken { fallback, ..deep_run::Taken::default() });
        if mode == PathMode::Compare {
            deep_run::record_compare(path, None, weight(&lexer));
        }
        return lexer;
    };
    let deep = deep_facts::extract(bytes, &cst, lang);
    if mode == PathMode::Compare {
        // No choice: both records are weighed and the audit decides nothing.
        deep_run::record_compare(path, Some((weight(&deep), cst.errors)), weight(&lexer));
        return deep;
    }
    if mode == PathMode::Auto {
        if loses(&lexer, &deep) {
            let fallback = Some("the CST record lost facts the lexer found".to_string());
            deep_run::record(path, deep_run::Taken { compared: true, fallback, ..deep_run::Taken::default() });
            return lexer;
        }
        let why = match weight(&deep).cmp(&weight(&lexer)) {
            std::cmp::Ordering::Greater => "",
            std::cmp::Ordering::Equal => "tie",
            std::cmp::Ordering::Less => {
                // The lexer read more than the CST did: keep it, and say so,
                // but this is not a fallback — nothing was lost.
                deep_run::record(path, deep_run::Taken { compared: true, why: "lexer", ..deep_run::Taken::default() });
                return lexer;
            }
        };
        deep_run::record(path, deep_run::Taken { deep: true, compared: true, errors: cst.errors, fallback: None, why });
        return deep;
    }
    deep_run::record(path, deep_run::Taken { deep: true, errors: cst.errors, fallback: None, why: "", ..deep_run::Taken::default() });
    deep
}
