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
            match resolver.call_at(&call.path, symbol, position, id, facts, scope) {
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
        /// Lexer artefacts accepted in place of a reproduction: a named call
        /// that is not in the source, or a statement read as an expression.
        pub(super) accepted: super::Accepted,
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
        /// Lexer calls with no name in the source (`.call`).
        accepted_anonymous_calls: u64,
        /// Lexer calls to a statement keyword (`assert`).
        accepted_keyword_calls: u64,
        /// Lexer `x` events for a statement keyword read as an expression.
        accepted_keyword_events: u64,
        /// Lexer assignment events whose target is an imported type name.
        accepted_annotation_targets: u64,
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
            let mut text = format!("rust-graph facts.meta 4 {}\n", bits());
            let mut rows: Vec<(&String, &Taken)> = self.records.iter().collect();
            rows.sort_unstable_by(|left, right| left.0.cmp(right.0));
            for (path, taken) in rows {
                let parser = match (taken.deep, taken.fallback.is_some()) {
                    (true, _) => "deep",
                    (false, true) => "lexer(fallback)",
                    (false, false) => "lexer",
                };
                text.push_str(&format!(
                    "{} {} {} {} {} {} {} {} {} {}\n",
                    escape(path),
                    parser,
                    taken.errors,
                    u8::from(taken.compared),
                    taken.why,
                    escape(taken.fallback.as_deref().unwrap_or("")),
                    taken.accepted.anonymous_calls,
                    taken.accepted.keyword_calls,
                    taken.accepted.keyword_events,
                    taken.accepted.annotation_targets
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
                self.accepted_anonymous_calls += u64::from(taken.accepted.anonymous_calls);
                self.accepted_keyword_calls += u64::from(taken.accepted.keyword_calls);
                self.accepted_keyword_events += u64::from(taken.accepted.keyword_events);
                self.accepted_annotation_targets += u64::from(taken.accepted.annotation_targets);
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
            )?;
            // Lexer artefacts accepted instead of reproduced. Zero everywhere
            // means the two readers agreed on every fact the guard compares;
            // a count here is a difference a reader can audit, not a silent
            // agreement.
            write!(
                out,
                ",\"accepted\":{{\"anonymous_call\":{},\"keyword_call\":{},\"keyword_event\":{}}}",
                self.accepted_anonymous_calls,
                self.accepted_keyword_calls,
                self.accepted_keyword_events
            )
        }

        pub(super) fn report_line(&self, text: &str) -> String {
            format!(
                "{text}\nParsed by: {} files through the CST, {} through the lexer, {} requested CST parses fell back.\nPaths: {{\"deep\": {}, \"lexer\": {}, \"chosen_deep\": {}, \"chosen_lexer\": {}}} (files with a CST record, files the lexer alone could take; which record was emitted per file).\nAccepted differences: {} lexer-only calls ({} anonymous, {} statement keywords), {} lexer-only value events for a statement keyword, {} lexer assignment targets naming an imported type instead of a binder.\n",
                self.deep,
                self.lexer,
                self.fallbacks,
                self.compared,
                self.lexer_only,
                self.deep,
                self.lexer,
                self.accepted_anonymous_calls + self.accepted_keyword_calls,
                self.accepted_anonymous_calls,
                self.accepted_keyword_calls,
                self.accepted_keyword_events,
                self.accepted_annotation_targets
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
        if lines.next()? != format!("rust-graph facts.meta 4 {}", bits()) {
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
            let reason = unescape(fields.next()?);
            let accepted = super::Accepted {
                anonymous_calls: fields.next()?.parse().ok()?,
                keyword_calls: fields.next()?.parse().ok()?,
                keyword_events: fields.next()?.parse().ok()?,
                annotation_targets: fields.next()?.parse().ok()?,
            };
            out.insert(
                path,
                Taken {
                    deep: parser == "deep",
                    compared,
                    errors,
                    fallback: (!reason.is_empty()).then_some(reason),
                    why,
                    accepted,
                },
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

/// The lexer facts a CST record is allowed not to reproduce.
///
/// The one-pass lexer has artefacts of its own: it names a call whose callee is
/// a value `.call`, it reads a statement keyword as an expression
/// (`assert (a, b) == c` becomes a call to `assert`), and it reads an
/// annotation's type as an assignment target (`x: Dict[str, int] = {}` binds
/// `Dict`). A fact layer must not invent those to agree with the lexer, so they
/// are accepted — and counted, so a reader sees what was set aside rather than
/// taking silence for agreement. Only a target that names a name the file
/// *imports* is read that way: a name the file binds is a binder, and a
/// statement whose names disagree for any other reason is a loss.
#[cfg(all(feature = "deep", feature = "parser"))]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Accepted {
    /// `f(x)(y)`: the callee is the value `f(x)`, so the lexer names the call
    /// `.call`. No such name is written in the source.
    pub(crate) anonymous_calls: u32,
    /// `assert (a, b) == c`: the lexer reads a statement keyword as a callee.
    pub(crate) keyword_calls: u32,
    /// An `x` event the lexer wrote because it read a keyword statement as an
    /// expression. The calls on that line are still compared through the call
    /// table, so only the value-flow event is set aside.
    pub(crate) keyword_events: u32,
    /// `x: Dict[str, int] = {...}`: the lexer reads the annotation's type name
    /// as the assignment target. The name is imported, not bound, and the CST
    /// wrote the statement's real target with the same value, so no binding was
    /// lost — the lexer invented one.
    pub(crate) annotation_targets: u32,
}

#[cfg(all(feature = "deep", feature = "parser"))]
impl Accepted {
    /// Everything set aside for this file, of every kind.
    pub(crate) fn total(self) -> u32 {
        self.anonymous_calls + self.keyword_calls + self.keyword_events + self.annotation_targets
    }
}

/// A statement keyword is never a callee, so a lexer call to one is an artefact
/// of reading the statement as an expression. Only a bare path counts:
/// `match.group` is an attribute of a name and a real call.
#[cfg(all(feature = "deep", feature = "parser"))]
const KEYWORD_STATEMENTS: [&str; 25] = [
    "assert", "return", "yield", "await", "throw", "raise", "del", "delete", "new", "sizeof", "defined", "if", "elif",
    "else", "for", "foreach", "while", "until", "unless", "switch", "try", "except", "finally", "with", "do",
];

/// A lexer call with no name in the source for the CST to read.
#[cfg(all(feature = "deep", feature = "parser"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phantom {
    /// `f(x)(y)`: the callee is a value, so the call is named `.call`.
    Anonymous,
    /// `assert (a, b) == c`: the callee is a statement keyword.
    Keyword,
}

#[cfg(all(feature = "deep", feature = "parser"))]
fn phantom_call(path: &str) -> Option<Phantom> {
    if path.is_empty() || path.starts_with('.') || path.ends_with('.') {
        return Some(Phantom::Anonymous);
    }
    (!path.contains('.') && KEYWORD_STATEMENTS.contains(&path)).then_some(Phantom::Keyword)
}

/// How the two records compare: a loss, or the lexer artefacts accepted in its
/// place. A loss keeps the lexer's record; an acceptance is counted and does
/// not decide anything.
#[cfg(all(feature = "deep", feature = "parser"))]
#[derive(Default, Clone, Debug)]
pub(crate) struct Verdict {
    loss: Option<&'static str>,
    /// The fact that tripped the check, in the reader's own words: the import,
    /// the call, the binding, or the event with both records' payloads. Empty
    /// when nothing was lost. Written on the fallback path only, so a file the
    /// CST keeps pays nothing for it.
    detail: String,
    accepted: Accepted,
}

#[cfg(all(feature = "deep", feature = "parser"))]
impl Verdict {
    pub(crate) fn lost(&self) -> bool {
        self.loss.is_some()
    }

    /// Which check fired, for a test or a reader of the report.
    pub(crate) fn why(&self) -> Option<&'static str> {
        self.loss
    }

    /// The fact behind `why`, empty when nothing was lost.
    pub(crate) fn detail(&self) -> &str {
        &self.detail
    }

    pub(crate) fn accepted(&self) -> Accepted {
        self.accepted
    }
}

/// Whether the CST record lost anything the lexer's found, and which lexer
/// artefacts were accepted instead.
///
/// The deep walk is the better reader where its tables cover a construct, but
/// where a grammar or a table has not reached one it can silently see less than
/// the one-pass lexer — `from m import *`, `a, b = f(x), 1`, `f"{x}"` are three
/// the differential fixtures do not carry. `auto` therefore compares the two
/// records and keeps the lexer's when the CST's is missing something: fewer
/// imports, a named symbol the lexer had, a call, a typed variable, or a flow
/// event. A loss is recorded as a fallback rather than hidden, and `--deep`
/// skips the comparison because the user asked for the CST.
///
/// Flow events are compared by *structure*, not by count: the same number of
/// `["r",...]` events can carry different values, and a value the lexer read and
/// the CST wrote as `0` is a loss even though both paths wrote the event. Each
/// event is reduced to its shape — marker, assignment targets, and the value
/// with the call table's numbering erased — and an event is covered when a deep
/// event with the same marker and targets contains every name and call the
/// lexer's value names. Association is not compared (the lexer's
/// `g<number>(x)` merge of names is covered by the CST's call `g(x)`), but a
/// name or a call the CST dropped is a loss even when the count and the shape
/// agree. Only a value the CST dropped to `0`, a target it did not write, and an
/// event it did not write at all are losses. What the guard cannot see — two
/// non-constant readings that disagree in content — is what the differential
/// harness has the source for.
///
/// What the lexer invented is never a loss and never forces a fallback: a call
/// whose path is not a name in the source (`.call`, an anonymous application), a
/// call to a statement keyword (`assert`), an `x` event for a statement keyword,
/// and an assignment target that names an imported type. They are counted in
/// `Accepted` by kind, so the report says how much was set aside.
#[cfg(all(feature = "deep", feature = "parser"))]
pub(crate) fn loses(lexer: &Facts, deep: &Facts) -> Verdict {
    let mut accepted = Accepted::default();
    let verdict = |loss: Option<&'static str>, detail: String, accepted: Accepted| Verdict { loss, detail, accepted };
    for import in &lexer.imports {
        let same = deep.imports.iter().any(|row| {
            row.module == import.module && row.name == import.name && row.alias == import.alias && row.kind == import.kind
        });
        if !same {
            let detail = format!("import {} name {} alias {}", import.module, import.name, import.alias);
            return verdict(Some("import"), detail, accepted);
        }
    }
    for symbol in lexer.symbols.iter().filter(|symbol| symbol.kind != KIND_MODULE) {
        let same = deep.symbols.iter().any(|row| row.name == symbol.name && row.kind == symbol.kind);
        if !same {
            let detail = format!("symbol {} kind {} line {}", symbol.name, symbol.kind, symbol.line);
            return verdict(Some("symbol"), detail, accepted);
        }
    }
    let name_of = |path: &str| path.rsplit('.').next().unwrap_or(path).to_string();
    for symbol in &lexer.symbols {
        for call in &symbol.calls {
            match phantom_call(&call.path) {
                Some(Phantom::Anonymous) => {
                    accepted.anonymous_calls += 1;
                    continue;
                }
                Some(Phantom::Keyword) => {
                    accepted.keyword_calls += 1;
                    continue;
                }
                None => {}
            }
            let name = name_of(&call.path);
            let near = |line: u32| (line as i64 - call.line as i64).abs() <= 1;
            let same = deep
                .symbols
                .iter()
                .flat_map(|row| row.calls.iter())
                .any(|row| name_of(&row.path) == name && near(row.line));
            if !same {
                let detail = format!("call {} line {} in {}", call.path, call.line, symbol.name);
                return verdict(Some("call"), detail, accepted);
            }
        }
    }
    // `repo = Repo()` is what ties `repo.find()` to `Repo.find`: the resolver's
    // typed-variable edges come from this table, so a binding the CST misses is
    // a lost fact even though no event names it.
    for pair in &lexer.vars {
        let wanted = lexer.vars.iter().filter(|row| *row == pair).count();
        let found = deep.vars.iter().filter(|row| *row == pair).count();
        if found < wanted {
            let detail = format!("var {} = {}", pair.0, pair.1);
            return verdict(Some("var"), detail, accepted);
        }
    }
    for symbol in &lexer.symbols {
        let paired = if symbol.kind == KIND_MODULE {
            deep.symbols.iter().find(|row| row.kind == KIND_MODULE)
        } else {
            deep.symbols.iter().find(|row| row.name == symbol.name && row.kind == symbol.kind)
        };
        let Some(paired) = paired else { continue };
        if let Some(detail) = uncovered(lexer, symbol, paired, &mut accepted) {
            return verdict(Some("event"), detail, accepted);
        }
    }
    verdict(None, String::new(), accepted)
}

/// The first flow event of `symbol` that no event of `paired` covers, with both
/// records' payloads so a fallback can be read rather than counted. The
/// acceptance counters are updated on the way past the same markers `loses`
/// skips, so there is one implementation of the comparison.
#[cfg(all(feature = "deep", feature = "parser"))]
fn uncovered(lexer: &Facts, symbol: &crate::facts::Symbol, paired: &crate::facts::Symbol, accepted: &mut Accepted) -> Option<String> {
    let keyword_lines: Vec<u32> = symbol
        .calls
        .iter()
        .filter(|call| phantom_call(&call.path) == Some(Phantom::Keyword))
        .map(|call| call.line)
        .collect();
    let mut wanted: Vec<(u8, Shape)> = Vec::new();
    for event in raw_events(&symbol.events) {
        if !matches!(event.tag, b'a' | b'r' | b'x') {
            continue;
        }
        if event.tag == b'x' && keyword_lines.contains(&event.line) {
            accepted.keyword_events += 1;
            continue;
        }
        wanted.push((event.tag, shape_event(event.tag, event.line, event.payload, &symbol.calls, &mut *accepted)));
    }
    let mut found: Vec<(u8, Shape)> = raw_events(&paired.events)
        .into_iter()
        .filter(|event| matches!(event.tag, b'a' | b'r' | b'x'))
        .map(|event| {
            let mut none = Accepted::default();
            (event.tag, shape_event(event.tag, event.line, event.payload, &paired.calls, &mut none))
        })
        .collect();
    for want in wanted {
        // The exact match is tried first and always wins: a statement both
        // readers bind the same way is never re-read as an artefact.
        let exact = found.iter().position(|row| row.0 == want.0 && covers(&row.1, &want.1).is_ok());
        if let Some(at) = exact {
            found.swap_remove(at);
            continue;
        }
        // A target list that names an imported type instead of a binder is the
        // lexer reading `x: Dict[...] = ...` as an assignment to `Dict`. Its
        // statement is still in the CST's record, with the real name and the
        // same value, so the fact is there and only the invented binder is set
        // aside.
        if annotation_targets(lexer, &want, &found) {
            accepted.annotation_targets += 1;
            if let Some(at) = found.iter().position(|row| {
                row.0 == want.0 && (row.1.line as i64 - want.1.line as i64).abs() <= 1 && covers_value(&row.1, &want.1).is_ok()
            }) {
                found.swap_remove(at);
            }
            continue;
        }
        // The candidate a reader would compare against: a nearby event with the
        // same marker and targets when there is one, so the reason names the
        // closest thing the CST wrote rather than an event on another statement.
        let near = found
            .iter()
            .find(|row| row.0 == want.0 && row.1.targets == want.1.targets && (row.1.line as i64 - want.1.line as i64).abs() <= 1)
            .or_else(|| found.iter().find(|row| row.0 == want.0 && (row.1.line as i64 - want.1.line as i64).abs() <= 1));
        let (other, why) = match near {
            Some(row) => {
                let miss = match covers(&row.1, &want.1) {
                    Ok(()) => Miss::Line,
                    Err(miss) => miss,
                };
                let why = match miss {
                    Miss::Line => "no event on a line both readers can mean",
                    Miss::Targets => "the statement's target list differs (names or order)",
                    Miss::Constant => "the CST wrote the value as the constant 0",
                    Miss::Name => "a name the lexer read is not in the CST's value",
                    Miss::Call => "a call or constant the lexer read is not in the CST's value",
                };
                (format!("line {} targets {} value {}", row.1.line, row.1.targets, row.1.value), why)
            }
            None => ("no event of that marker within one line".to_string(), "no event on a line both readers can mean"),
        };
        return Some(format!(
            "event {} line {} targets {} in {} ({why}): lexer {}; CST {}",
            want.0 as char, want.1.line, want.1.targets, symbol.name, want.1.value, other
        ));
    }
    None
}

/// Whether a lexer assignment event names an imported type rather than a binder,
/// and the CST wrote the same statement with a value that covers it.
#[cfg(all(feature = "deep", feature = "parser"))]
fn annotation_targets(lexer: &Facts, want: &(u8, Shape), found: &[(u8, Shape)]) -> bool {
    if want.0 != b'a' {
        return false;
    }
    let names = quoted_names(&want.1.targets);
    if names.is_empty() {
        return false;
    }
    let imported = |name: &str| {
        lexer.imports.iter().any(|row| {
            let local = if row.alias.is_empty() { row.name.as_str() } else { row.alias.as_str() };
            local == name
        })
    };
    if !names.iter().all(|name| imported(name)) {
        return false;
    }
    found.iter().any(|row| {
        row.0 == b'a'
            && (row.1.line as i64 - want.1.line as i64).abs() <= 1
            && row.1.targets != want.1.targets
            && covers_value(&row.1, &want.1).is_ok()
    })
}

/// The quoted names in a target list: `["a","b"]` yields `a`, `b`.
#[cfg(all(feature = "deep", feature = "parser"))]
fn quoted_names(text: &str) -> Vec<&str> {
    text.split('"').skip(1).step_by(2).filter(|name| !name.is_empty()).collect()
}

/// What one flow event says, reduced to what the lexer and the CST must agree
/// on: the assignment targets (empty for `r`/`x`) and the value's form with the
/// call table's numbering erased.
#[cfg(all(feature = "deep", feature = "parser"))]
struct Shape {
    line: u32,
    targets: String,
    value: String,
}

/// Why a deep event does not cover a lexer event. A reason, not a sentence: the
/// caller has the payloads and writes the line a reader sees.
#[cfg(all(feature = "deep", feature = "parser"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Miss {
    /// No event of that marker on a line the two readers can both mean.
    Line,
    /// The statement binds different names.
    Targets,
    /// The CST wrote the value as the constant `0`.
    Constant,
    /// A name the lexer read is not in the CST's value.
    Name,
    /// A call the lexer read is not in the CST's value.
    Call,
}

/// Whether a deep event covers a lexer event with the same marker.
///
/// The value is compared by *containment*, not by association: both payloads
/// are flattened to the atoms they name (a name, a keyword's name, a call, a
/// constant), and every name and keyword the lexer read must be in the CST's
/// value, with at least as many calls and constants. Association does not
/// survive the flattening. A name or a call the CST dropped is a loss even when
/// the event count and the shape both agree; a literal may be dropped, since the
/// one-pass lexer drops them in call arguments too. A value the CST wrote as the
/// constant `0` is the loss this guard exists for: the grammar keeps `f"{a}"` as
/// one token, so no field is left to read.
///
/// The one name that may be missing is a type argument: the lexer reads
/// `g<number>(x)` as a merge of three names and the CST as a call to `g` with one
/// argument, so `number` is a token the one-pass reader flattened into a name,
/// not a value the call carries. That tolerance is exactly a merge read as a
/// call — nothing else is excused, so a call the lexer read and the CST read as
/// a name is still a loss.
///
/// The targets are compared as written: a `for`/assignment target list is the
/// set of names the statement binds, and a reader of the record is told which
/// names by it. The CST reversing them is a difference this guard reports, not
/// one it papers over.
#[cfg(all(feature = "deep", feature = "parser"))]
fn covers(deep: &Shape, lexer: &Shape) -> Result<(), Miss> {
    // A lexer statement and the CST node it sits on can disagree by a line (a
    // signature stamped where it ends against where it starts), so an event is
    // paired near its line, never across the file: a duplicate target on
    // another line must not stand in for the event that lost its value.
    if (deep.line as i64 - lexer.line as i64).abs() > 1 {
        return Err(Miss::Line);
    }
    if deep.targets != lexer.targets {
        return Err(Miss::Targets);
    }
    covers_value(deep, lexer)
}

#[cfg(all(feature = "deep", feature = "parser"))]
fn covers_value(deep: &Shape, lexer: &Shape) -> Result<(), Miss> {
    if lexer.value == "0" {
        return Ok(());
    }
    if deep.value == "0" {
        return Err(Miss::Constant);
    }
    let mut wanted = Vec::new();
    let mut found = Vec::new();
    atoms(&lexer.value, &mut wanted);
    atoms(&deep.value, &mut found);
    let mut missing: Vec<&String> = Vec::new();
    for name in wanted.iter().filter(|atom| atom.starts_with("n:") || atom.starts_with("k:")) {
        match found.iter().position(|row| row == name) {
            Some(at) => {
                found.swap_remove(at);
            }
            None => missing.push(name),
        }
    }
    if !missing.is_empty() {
        // A merge the CST read as a call: the lexer flattened a call's tokens
        // into names, and its type arguments are not in the call's record. A
        // keyword name is never excused — both readers write a keyword's name
        // inside the call, so one missing there is a loss.
        let merged_as_call = matches!(value_form(&lexer.value), "m" | "t") && value_form(&deep.value) == "c";
        if !merged_as_call || missing.iter().any(|name| name.starts_with("k:")) {
            return Err(Miss::Name);
        }
    }
    for (kind, miss) in [("call", Miss::Call), ("0", Miss::Call)] {
        let wanted = wanted.iter().filter(|atom| *atom == kind).count();
        let found = found.iter().filter(|atom| *atom == kind).count();
        if found < wanted {
            return Err(miss);
        }
    }
    Ok(())
}

/// The top-level form of a value: `n` (a name), `c` (a call), `t`/`m` (a list or
/// a merge), `k` (a keyword argument), `0` (a constant).
#[cfg(all(feature = "deep", feature = "parser"))]
fn value_form(value: &str) -> &str {
    let end = value.find([':', '(', '[']).unwrap_or(value.len());
    &value[..end]
}

/// Every atom inside a shape, flattened through the combining forms: names and
/// keyword names as themselves, calls as `call`, constants as `0`. Association
/// does not survive the flattening, so `a and b and c` read as one merge and as
/// nested merges compares equal.
#[cfg(all(feature = "deep", feature = "parser"))]
fn atoms(shape: &str, out: &mut Vec<String>) {
    if let Some(rest) = shape.strip_prefix("c(") {
        out.push("call".to_string());
        let inner = rest.strip_suffix(')').unwrap_or(rest);
        if let Some((receiver, arguments)) = inner.split_once(';') {
            atoms(receiver, out);
            let arguments = arguments.strip_prefix('[').unwrap_or(arguments);
            let arguments = arguments.strip_suffix(']').unwrap_or(arguments);
            for item in split_items(arguments) {
                atoms(item, out);
            }
        }
        return;
    }
    if let Some(rest) = shape.strip_prefix("m:[").or_else(|| shape.strip_prefix("t:[")) {
        let inner = rest.strip_suffix(']').unwrap_or(rest);
        for item in split_items(inner) {
            atoms(item, out);
        }
        return;
    }
    if let Some(rest) = shape.strip_prefix("k(") {
        if let Some((name, value)) = rest.split_once("):") {
            out.push(format!("k:{name}"));
            atoms(value, out);
            return;
        }
    }
    out.push(shape.to_string());
}

/// Split a shape list at commas that are not inside a nested shape.
#[cfg(all(feature = "deep", feature = "parser"))]
fn split_items(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (at, byte) in text.bytes().enumerate() {
        match byte {
            b'[' | b'(' | b'{' => depth += 1,
            b']' | b')' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                out.push(&text[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out.retain(|item| !item.is_empty());
    out
}

/// One top-level element of an events array: its marker letter, its line, and
/// the text after the line. `payload` is the targets and value of an `a` event,
/// or the value of an `r`/`x` event; control markers are parsed but skipped by
/// the caller because they carry no value.
#[cfg(all(feature = "deep", feature = "parser"))]
struct RawEvent<'a> {
    tag: u8,
    line: u32,
    payload: &'a str,
}

#[cfg(all(feature = "deep", feature = "parser"))]
fn raw_events(events: &str) -> Vec<RawEvent<'_>> {
    let bytes = events.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'[' {
            at += 1;
            continue;
        }
        let end = element_end(bytes, at);
        if let Some(event) = event_at(events, at, end) {
            out.push(event);
        }
        at = end.max(at + 1);
    }
    out
}

/// The index just past the `]` that closes the `[` at `start`.
#[cfg(all(feature = "deep", feature = "parser"))]
fn element_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    let mut at = start;
    while at < bytes.len() {
        match bytes[at] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return at + 1;
                }
            }
            _ => {}
        }
        at += 1;
    }
    bytes.len()
}

#[cfg(all(feature = "deep", feature = "parser"))]
fn event_at(events: &str, start: usize, end: usize) -> Option<RawEvent<'_>> {
    let bytes = events.as_bytes();
    if end < start + 6 || bytes[start] != b'[' || bytes[start + 1] != b'"' || bytes[start + 3] != b'"' {
        return None;
    }
    if bytes[start + 4] != b',' {
        return None;
    }
    let tag = bytes[start + 2];
    let mut at = start + 5;
    let mut line = 0u32;
    while at < end && bytes[at].is_ascii_digit() {
        line = line.saturating_mul(10).saturating_add((bytes[at] - b'0') as u32);
        at += 1;
    }
    if at >= end || bytes[at] != b',' {
        return None;
    }
    Some(RawEvent { tag, line, payload: &events[at + 1..end - 1] })
}

#[cfg(all(feature = "deep", feature = "parser"))]
fn shape_event(tag: u8, line: u32, payload: &str, calls: &[crate::facts::Call], accepted: &mut Accepted) -> Shape {
    if tag != b'a' {
        let mut pos = 0;
        return Shape { line, targets: String::new(), value: shape_at(payload, &mut pos, calls, accepted) };
    }
    let targets_end = element_end(payload.as_bytes(), 0);
    let targets = payload[..targets_end].to_string();
    let mut pos = targets_end;
    if payload.as_bytes().get(pos) == Some(&b',') {
        pos += 1;
    }
    let value = shape_at(payload, &mut pos, calls, accepted);
    Shape { line, targets, value }
}

/// The form of one value at `pos`, with call numbering erased: a call
/// contributes its receiver and arguments, never its symbol-local index. An
/// anonymous call (`f(x)(y)` is named `.call`) contributes the value it applies;
/// the application has no name for the CST to read, so it is not a fact either
/// path can be asked to agree on.
#[cfg(all(feature = "deep", feature = "parser"))]
fn shape_at(text: &str, pos: &mut usize, calls: &[crate::facts::Call], accepted: &mut Accepted) -> String {
    let bytes = text.as_bytes();
    if *pos >= bytes.len() {
        return "0".to_string();
    }
    if bytes[*pos] == b'0' {
        *pos += 1;
        return "0".to_string();
    }
    if bytes[*pos] != b'[' {
        *pos += 1;
        return String::new();
    }
    let start = *pos;
    let end = element_end(bytes, start);
    *pos = end;
    let body = &text[start..end];
    if body.len() < 7 || body.as_bytes()[1] != b'"' {
        return body.to_string();
    }
    let tag = body.as_bytes()[2];
    let inner = &body[4..body.len() - 1];
    let inner_bytes = inner.as_bytes();
    match tag {
        b'n' => format!("n:{}", inner.trim_start_matches(',').trim_matches('"')),
        b'c' => {
            let mut at = 1;
            let comma = inner[at..].find(',').map(|value| value + at).unwrap_or(inner.len());
            let id: usize = inner[at..comma].parse().unwrap_or(usize::MAX);
            at = comma + 1;
            let receiver = shape_at(inner, &mut at, calls, accepted);
            if inner_bytes.get(at) == Some(&b',') {
                at += 1;
            }
            let mut arguments = Vec::new();
            if inner_bytes.get(at) == Some(&b'[') {
                let list_end = element_end(inner_bytes, at);
                let list = &inner[at + 1..list_end.saturating_sub(1)];
                let mut item = 0;
                while item < list.len() {
                    if list.as_bytes()[item] == b'[' || list.as_bytes()[item] == b'0' {
                        arguments.push(shape_at(list, &mut item, calls, accepted));
                    } else {
                        item += 1;
                    }
                }
            }
            let path = calls.get(id).map(|call| call.path.as_str()).unwrap_or("");
            if phantom_call(path) == Some(Phantom::Anonymous) {
                return receiver;
            }
            format!("c({receiver};[{}])", arguments.join(","))
        }
        b't' | b'm' => {
            let mut at = 0;
            while at < inner_bytes.len() && inner_bytes[at] != b'[' {
                at += 1;
            }
            let mut items = Vec::new();
            if at < inner_bytes.len() {
                at += 1;
                let stop = inner_bytes.len().saturating_sub(1);
                while at < stop {
                    if inner_bytes[at] == b'[' || inner_bytes[at] == b'0' {
                        items.push(shape_at(inner, &mut at, calls, accepted));
                    } else {
                        at += 1;
                    }
                }
            }
            format!("{}:[{}]", tag as char, items.join(","))
        }
        b'k' => {
            let mut at = 0;
            let mut name = String::new();
            if inner_bytes.first() == Some(&b',') {
                at += 1;
            }
            if inner_bytes.get(at) == Some(&b'"') {
                let close = inner[at + 1..].find('"').map(|value| value + at + 1).unwrap_or(inner.len());
                name = inner[at + 1..close].to_string();
                at = close + 1;
            }
            if inner_bytes.get(at) == Some(&b',') {
                at += 1;
            }
            format!("k({name}):{}", shape_at(inner, &mut at, calls, accepted))
        }
        _ => body.to_string(),
    }
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
        let verdict = loses(&lexer, &deep);
        if verdict.lost() {
            let fallback = Some("the CST record lost facts the lexer found".to_string());
            deep_run::record(
                path,
                deep_run::Taken { compared: true, fallback, accepted: verdict.accepted(), ..deep_run::Taken::default() },
            );
            return lexer;
        }
        let why = match weight(&deep).cmp(&weight(&lexer)) {
            std::cmp::Ordering::Greater => "",
            std::cmp::Ordering::Equal => "tie",
            std::cmp::Ordering::Less => {
                // The lexer read more than the CST did: keep it, and say so,
                // but this is not a fallback — nothing was lost.
                deep_run::record(
                    path,
                    deep_run::Taken { compared: true, why: "lexer", accepted: verdict.accepted(), ..deep_run::Taken::default() },
                );
                return lexer;
            }
        };
        deep_run::record(
            path,
            deep_run::Taken { deep: true, compared: true, errors: cst.errors, fallback: None, why, accepted: verdict.accepted() },
        );
        return deep;
    }
    deep_run::record(path, deep_run::Taken { deep: true, errors: cst.errors, fallback: None, why: "", ..deep_run::Taken::default() });
    deep
}
