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
#[path = "deep_facts/mod.rs"]
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
        /// Lexer symbols with no name of their own (`<anonymous>@12`).
        accepted_anonymous_symbols: u64,
        /// Named symbols the lexer built out of an ordinary expression.
        accepted_invented_symbols: u64,
        /// Lexer calls that are a pattern in the source (`Some(x) =>`).
        accepted_positional_calls: u64,
        /// Lexer events on a line inside a macro body.
        accepted_positional_events: u64,
        /// Lexer symbols on a line inside a macro body.
        accepted_positional_symbols: u64,
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
            let mut text = format!("rust-graph facts.meta 7 {}\n", bits());
            let mut rows: Vec<(&String, &Taken)> = self.records.iter().collect();
            rows.sort_unstable_by(|left, right| left.0.cmp(right.0));
            for (path, taken) in rows {
                let parser = match (taken.deep, taken.fallback.is_some()) {
                    (true, _) => "deep",
                    (false, true) => "lexer(fallback)",
                    (false, false) => "lexer",
                };
                text.push_str(&format!(
                    "{} {} {} {} {} {} {} {} {} {} {} {} {} {} {}\n",
                    escape(path),
                    parser,
                    taken.errors,
                    u8::from(taken.compared),
                    taken.why,
                    escape(taken.fallback.as_deref().unwrap_or("")),
                    taken.accepted.anonymous_calls,
                    taken.accepted.keyword_calls,
                    taken.accepted.keyword_events,
                    taken.accepted.annotation_targets,
                    taken.accepted.anonymous_symbols,
                    taken.accepted.invented_symbols,
                    taken.accepted.positional_calls,
                    taken.accepted.positional_events,
                    taken.accepted.positional_symbols
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
                ",\"accepted\":{{\"anonymous_call\":{},\"keyword_call\":{},\"keyword_event\":{},\"annotation_target\":{},\"anonymous_symbol\":{},\"invented_symbol\":{},\"positional_call\":{},\"positional_event\":{},\"positional_symbol\":{}}}",
                self.accepted_anonymous_calls,
                self.accepted_keyword_calls,
                self.accepted_keyword_events,
                self.accepted_annotation_targets,
                self.accepted_anonymous_symbols,
                self.accepted_invented_symbols,
                self.accepted_positional_calls,
                self.accepted_positional_events,
                self.accepted_positional_symbols
            )
        }

        pub(super) fn report_line(&self, text: &str) -> String {
            format!(
                "{text}\nParsed by: {} files through the CST, {} through the lexer, {} requested CST parses fell back.\nPaths: {{\"deep\": {}, \"lexer\": {}, \"chosen_deep\": {}, \"chosen_lexer\": {}}} (files with a CST record, files the lexer alone could take; which record was emitted per file).\nAccepted differences: {} lexer-only calls ({} anonymous, {} statement keywords), {} lexer-only value events for a statement keyword, {} lexer assignment targets naming an imported type instead of a binder, {} lexer symbols with no name in the source, {} lexer symbols whose line declares nothing, {} lexer calls the source writes as a pattern, an attribute or a macro body, {} lexer events and {} lexer symbols on a line inside one.\n",
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
                self.accepted_annotation_targets,
                self.accepted_anonymous_symbols,
                self.accepted_invented_symbols,
                self.accepted_positional_calls,
                self.accepted_positional_events,
                self.accepted_positional_symbols
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
        if lines.next()? != format!("rust-graph facts.meta 7 {}", bits()) {
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
                anonymous_symbols: fields.next()?.parse().ok()?,
                invented_symbols: fields.next()?.parse().ok()?,
                positional_calls: fields.next()?.parse().ok()?,
                positional_events: fields.next()?.parse().ok()?,
                positional_symbols: fields.next()?.parse().ok()?,
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
    /// A symbol the lexer opened with no name of its own (`<anonymous>@12`, or
    /// an empty one for `go func() {`). There is nothing in the source for the
    /// CST to name, so requiring it would be requiring an invention.
    pub(crate) anonymous_symbols: u32,
    /// A named symbol the lexer built out of an ordinary expression: the line it
    /// points at carries no definition keyword for that name.
    pub(crate) invented_symbols: u32,
    /// A call the lexer read out of a position that is not code: a pattern
    /// (`Some(x) =>`), an attribute (`#[cfg(...)]`) or a macro body, where the
    /// tokens are data for the macro and the CST is right to record no call.
    pub(crate) positional_calls: u32,
    /// A lexer event whose line is inside a macro body.
    pub(crate) positional_events: u32,
    /// A lexer symbol whose line is inside a macro body.
    pub(crate) positional_symbols: u32,
}

#[cfg(all(feature = "deep", feature = "parser"))]
impl Accepted {
    /// Everything set aside for this file, of every kind.
    pub(crate) fn total(self) -> u32 {
        self.anonymous_calls
            + self.keyword_calls
            + self.keyword_events
            + self.annotation_targets
            + self.anonymous_symbols
            + self.invented_symbols
            + self.positional_calls
            + self.positional_events
            + self.positional_symbols
    }
}

/// A statement keyword is never a callee, so a lexer call to one is an artefact
/// of reading the statement as an expression. Only a bare path counts:
/// `match.group` is an attribute of a name and a real call.
#[cfg(all(feature = "deep", feature = "parser"))]
const KEYWORD_STATEMENTS: [&str; 26] = [
    "assert", "return", "yield", "await", "throw", "raise", "del", "delete", "new", "sizeof", "defined", "if", "elif",
    "else", "for", "foreach", "while", "until", "unless", "switch", "try", "except", "finally", "with", "do",
    "match",
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
    loses_in(lexer, deep, "")
}

/// The same comparison, with the source in hand so a symbol can be checked
/// against the line it claims to come from. A one-pass lexer invents definitions
/// out of ordinary expressions (`for _, x := range []string{…}`, `if err := f()`,
/// `go func() {`), and the CST cannot reproduce a definition the source does not
/// contain. Without the source the check has nothing to appeal to.
#[cfg(all(feature = "deep", feature = "parser"))]
pub(crate) fn loses_in(lexer: &Facts, deep: &Facts, src: &str) -> Verdict {
    let mut accepted = Accepted::default();
    let macro_lines = macro_body_lines(src);
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
        // A symbol with no name is not a fact: the one-pass lexer opens one for
        // `go func() {`, for a function literal, for an `if` header it misreads
        // as a definition. The CST cannot reproduce a name the source does not
        // contain, so asking it to is a demand to invent one.
        if macro_lines
            .get(symbol.line.saturating_sub(1) as usize)
            .copied()
            .unwrap_or(false)
        {
            // The line is inside a macro body: its tokens are data for the
            // macro, and a definition read out of them is not in the source.
            accepted.positional_symbols += 1;
            continue;
        }
        if symbol.name.is_empty() || symbol.name.starts_with("<anonymous>@") {
            accepted.anonymous_symbols += 1;
            continue;
        }
        // The lexer's line says `for … := range []string{…}`; a definition of
        // `string` is not there. A fact the source does not support is not a
        // loss when the CST does not repeat it.
        if !src.is_empty() && !claims_the_line(src, &lexer.tag, symbol.line, &symbol.name) {
            accepted.invented_symbols += 1;
            continue;
        }
        // `mod tests { fn f() }`: the CST spells the symbol `tests.f` because the
        // module is its owner; the one-pass lexer files it under `f`. Same
        // definition, two spellings, so a dotted suffix counts.
        let same = deep.symbols.iter().any(|row| {
            row.kind == symbol.kind
                && (row.name == symbol.name || row.name.ends_with(&format!(".{}", symbol.name)))
        });
        if !same {
            let detail = format!("symbol {} kind {} line {}", symbol.name, symbol.kind, symbol.line);
            return verdict(Some("symbol"), detail, accepted);
        }
    }
    let name_of = |path: &str| path.rsplit('.').next().unwrap_or(path).to_string();
    for symbol in &lexer.symbols {
        for call in &symbol.calls {
            // A keyword is never a callee: `go func() {` in Go, `synchronized
            // (lock)` in Java. The lexer reads one as a call, the tree cannot
            // name it, and that is not a lost fact.
            if is_keyword_callee(&lexer.tag, &name_of(&call.path)) {
                accepted.keyword_calls += 1;
                continue;
            }
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
                // `match self.tok(i) { Some(t) => ... }`: the lexer reads the
                // arm's pattern as a call, and a pattern is not a call. The
                // source line says so — the name, its argument list, then `=>`.
                if !src.is_empty() && is_positional_call(src, &macro_lines, &name, call.line) {
                    accepted.positional_calls += 1;
                    continue;
                }
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
        if symbol.kind != KIND_MODULE {
            if macro_lines
                .get(symbol.line.saturating_sub(1) as usize)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            if symbol.name.is_empty() || symbol.name.starts_with("<anonymous>@") {
                continue;
            }
            if !src.is_empty() && !claims_the_line(src, &lexer.tag, symbol.line, &symbol.name) {
                continue;
            }
        }
        let paired = if symbol.kind == KIND_MODULE {
            deep.symbols.iter().find(|row| row.kind == KIND_MODULE)
        } else {
            // A method is named `__init__` in every class of the file, so the
            // owner decides which one is the same symbol. Without it a class's
            // events were compared against another class's and the file was
            // blamed for an event that was in the record.
            deep.symbols
                .iter()
                .find(|row| {
                    row.name == symbol.name
                        && row.kind == symbol.kind
                        && (symbol.owner.is_empty() || row.owner.is_empty() || row.owner == symbol.owner)
                })
        };
        let Some(paired) = paired else { continue };
        // The lexer does not open a symbol for `const fn` or for a closure, so a
        // whole impl body can land in the module's symbol while the CST files it
        // under the method. For the module, every symbol's events are the
        // module's: the fact is in the file either way.
        let extra: &[crate::facts::Symbol] = if symbol.kind == KIND_MODULE { &deep.symbols } else { &[] };
        if let Some(detail) = uncovered(lexer, symbol, paired, &mut accepted, src, &macro_lines, extra) {
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
fn uncovered(
    lexer: &Facts,
    symbol: &crate::facts::Symbol,
    paired: &crate::facts::Symbol,
    accepted: &mut Accepted,
    src: &str,
    macro_lines: &[bool],
    extra: &[crate::facts::Symbol],
) -> Option<String> {
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
        if macro_lines.get(event.line.saturating_sub(1) as usize).copied().unwrap_or(false)
            || is_attribute_call(src, event.line)
            || is_declaration_line(src, event.line)
            // `Bound(Bound),` in an enum body: the variants are declarations,
            // and the lexer reads the bare ones as a tuple value of its own.
            || is_variant_declaration(src, event.line)
            || is_pattern_line(src, event.line)
            || (event.tag == b'x' && is_let_else_line(src, event.line))
            || (event.tag == b'a' && is_control_cast_line(src, event.line))
            || (event.tag == b'x' && is_struct_pattern_condition(src, event.line))
            || (event.tag == b'x' && is_else_if_let_line(src, event.line))
            || (event.tag == b'x' && is_closure_block_line(src, event.line))
            || (event.tag == b'a' && is_control_header_line(src, event.line))
        {
            accepted.positional_events += 1;
            continue;
        }
        if event.tag == b'x' && keyword_lines.contains(&event.line) {
            accepted.keyword_events += 1;
            continue;
        }
        wanted.push((event.tag, shape_event(event.tag, event.line, event.payload, &symbol.calls, &mut *accepted, src, macro_lines)));
    }
    let mut found: Vec<(u8, Shape)> = raw_events(&paired.events)
        .into_iter()
        .filter(|event| matches!(event.tag, b'a' | b'r' | b'x'))
        .map(|event| {
            let mut none = Accepted::default();
            (event.tag, shape_event(event.tag, event.line, event.payload, &paired.calls, &mut none, src, macro_lines))
        })
        .collect();
    for other in extra.iter().filter(|row| row.name != paired.name || row.owner != paired.owner) {
        for event in raw_events(&other.events) {
            if !matches!(event.tag, b'a' | b'r' | b'x') {
                continue;
            }
            let mut none = Accepted::default();
            found.push((event.tag, shape_event(event.tag, event.line, event.payload, &other.calls, &mut none, src, macro_lines)));
        }
    }
    // The exact matches are taken first, all of them, before any statement is
    // re-read as an artefact. Pairing greedily one want at a time let a
    // resembling event on the next line consume the row a later want needed,
    // and the file was blamed for an event that was in the record all along.
    let mut leftover: Vec<(u8, Shape)> = Vec::new();
    for want in wanted {
        let exact = found
            .iter()
            .position(|row| row.0 == want.0 && row.1.line == want.1.line && covers(&row.1, &want.1, src).is_ok())
            .or_else(|| found.iter().position(|row| row.0 == want.0 && covers(&row.1, &want.1, src).is_ok()))
            .or_else(|| {
                if want.0 == b'x' && want.1.targets.is_empty() {
                    found.iter().position(|row| {
                        row.0 == b'a'
                            && (row.1.line as i64 - want.1.line as i64).abs() <= 1
                            && covers_value(&row.1, &want.1, src).is_ok()
                    })
                } else {
                    None
                }
            });
        match exact {
            Some(at) => {
                found.swap_remove(at);
            }
            None => leftover.push(want),
        }
    }
    for want in leftover {
        // A target list that names an imported type instead of a binder is the
        // lexer reading `x: Dict[...] = ...` as an assignment to `Dict`. Its
        // statement is still in the CST's record, with the real name and the
        // same value, so the fact is there and only the invented binder is set
        // aside.
        if annotation_targets(lexer, &want, &found, src) || type_as_target(src, &want, &found) {
            accepted.annotation_targets += 1;
            // Both checks require a covering row before they say yes, so the
            // statement is in the record either way; the row is retired when it
            // is still there so a second invented binder cannot claim it.
            if let Some(at) = found.iter().position(|row| {
                row.0 == want.0 && (row.1.line as i64 - want.1.line as i64).abs() <= 1 && covers_value(&row.1, &want.1, src).is_ok()
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
                let miss = match covers(&row.1, &want.1, src) {
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


/// Whether the call the lexer reports on this line is written as a pattern:
/// `<name>(… )` followed by `=>`. A match arm's pattern binds, it does not call,
/// and the CST records no call for it.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_positional_call(src: &str, macro_lines: &[bool], name: &str, line: u32) -> bool {
    is_attribute_call(src, line)
        || is_declaration_call(src, name, line)
        || is_visibility_call(src, line, name)
        || is_variant_declaration(src, line)
        || macro_lines.get(line.saturating_sub(1) as usize).copied().unwrap_or(false)
        || is_pattern_call(src, name, line)
}

/// `pub(crate) fn f()`: `pub` is a visibility marker, not a callee. The lexer
/// reads `pub(` as a call and the CST records none, which is a difference in
/// spelling rather than a lost fact.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_visibility_call(src: &str, line: u32, name: &str) -> bool {
    name == "pub"
        && src
            .lines()
            .nth(line.saturating_sub(1) as usize)
            .is_some_and(|text| text.trim_start().starts_with("pub("))
}

/// `Open(u8),` in an enum body declares a variant whose payload is a type: the
/// line is a declaration, and the parenthesis holds a type name, not arguments.
/// The trailing comma is what separates it from a tail expression `Some(x)`.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_variant_declaration(src: &str, line: u32) -> bool {
    let Some(text) = src.lines().nth(line.saturating_sub(1) as usize) else {
        return false;
    };
    let text = text.trim();
    if !text.ends_with("),") {
        return false;
    }
    let Some(open) = text.find('(') else {
        return false;
    };
    let name = text[..open].trim();
    let payload = &text[open + 1..text.len() - 2];
    !name.is_empty()
        && name.chars().next().is_some_and(char::is_uppercase)
        && !name.contains(|ch: char| !ch.is_alphanumeric() && ch != '_')
        && !payload.is_empty()
        && payload
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | ' ' | ',' | '<' | '>' | ':' | '&' | '[' | ']' | '\''))
}

/// `if char in ("'", '"'):` and `while x in seen:`: a membership test in a
/// control header reads to the one-pass lexer as a statement bound to the name
/// on the left, and the CST records the condition as an expression. The line is
/// a control header, so there is no binder to lose.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_control_header_line(src: &str, line: u32) -> bool {
    let Some(text) = src.lines().nth(line.saturating_sub(1) as usize) else {
        return false;
    };
    let text = text.trim_start();
    ["if ", "elif ", "while ", "for "].iter().any(|head| text.starts_with(head)) && text.contains(" in ")
}

/// `#[cfg(feature = "deep")]`, `#![allow(dead_code)]`: the tokens inside an
/// attribute are not a call, and the lexer reads `cfg` and `allow` as ones.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_attribute_call(src: &str, line: u32) -> bool {
    src.lines()
        .nth(line.saturating_sub(1) as usize)
        .map(|text| {
            // An attribute may sit mid-line: `pub field: u64, #[cfg(..)] pub
            // other: Mode,` is one line with an attribute in the middle, and
            // the lexer reads `cfg(` as a call on it.
            let text = text.trim_start();
            let attribute = text.starts_with("#[") || text.starts_with("#![") || text.contains(" #[");
            attribute && text.contains(']')
        })
        .unwrap_or(false)
}

/// Whether this line opens a declaration: `pub fn`, `struct`, `enum`, `def`,
/// `function`, `class`. A statement event there is the lexer reading the header
/// as an expression — `pub fn` becomes a merge of two names — and the CST is
/// right to record no expression at all.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_declaration_line(src: &str, line: u32) -> bool {
    // The header is what is written before the name's parenthesis or the body's
    // brace: `pub const fn new`, `pub(crate) struct Cache`, `def parse`.
    const INTRODUCERS: [&str; 13] = [
        "fn", "func", "def", "struct", "enum", "trait", "union", "type", "impl", "class", "interface", "module",
        // `pub mod budget;` is a declaration too, and the lexer reads the two
        // words as an expression.
        "mod",
    ];
    let Some(text) = src.lines().nth(line.saturating_sub(1) as usize) else {
        return false;
    };
    let text = text.trim_start();
    // `pub(super) struct Rng(u64);`: the visibility's own parentheses come
    // before the introducer, so splitting at the first `(` hid the word that
    // says the line is a declaration at all.
    let text = match text.strip_prefix("pub") {
        Some(rest) => rest
            .strip_prefix('(')
            .and_then(|rest| rest.split_once(')'))
            .map(|(_, tail)| tail)
            .unwrap_or(rest),
        None => text,
    };
    let header = text.split(['(', '{', '=']).next().unwrap_or(text);
    header.split_whitespace().any(|word| INTRODUCERS.contains(&word))
}

/// A match arm header: `pat => {`, `pat => {}`, or `pat =>` opening a block.
/// The pattern is not an expression statement, and the CST is right to record
/// no statement event for it.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_pattern_line(src: &str, line: u32) -> bool {
    let Some(text) = src.lines().nth(line.saturating_sub(1) as usize) else {
        return false;
    };
    let text = text.trim_start();
    let Some((pat, _tail)) = text.split_once("=>") else {
        return false;
    };
    if pat.contains(';') {
        return false;
    }
    let mut depth = 0i32;
    for ch in pat.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ => {}
        }
    }
    if depth != 0 {
        return false;
    }
    true
}

/// `let Pat = expr else {`: the lexer reads the header as an expression
/// statement that merges the value and the `else` block, while the CST records
/// the binding and visits the block's statements.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_let_else_line(src: &str, line: u32) -> bool {
    let Some(text) = src.lines().nth(line.saturating_sub(1) as usize) else {
        return false;
    };
    let text = text.trim_start();
    if !text.starts_with("let ") {
        return false;
    }
    let text = text.split("//").next().unwrap_or("").trim_end();
    text.contains(" else ")
}

/// An `if` or `while` header with a type cast: `if self.pair[i as usize] == NONE {`.
/// The one-pass lexer reads `as` inside the header as Python's `with ... as target:`
/// and files an assignment event to the comparison operand. The CST correctly
/// records the condition as an expression statement.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_control_cast_line(src: &str, line: u32) -> bool {
    let Some(text) = src.lines().nth(line.saturating_sub(1) as usize) else {
        return false;
    };
    let text = text.trim_start();
    (text.starts_with("if ") || text.starts_with("while "))
        && text.contains(" as ")
        && text.split("//").next().unwrap_or("").trim_end().ends_with('{')
}

/// An `if let` or `while let` with a struct pattern:
/// `if let Event::Start { forward_parent, .. } = self.events[...] {`.
/// The one-pass lexer reads `{` in the pattern as opening the `if` body,
/// emits the pattern fields as expression statements inside a fake block,
/// and closes it at `}`, before reading the remainder of the line.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_struct_pattern_condition(src: &str, line: u32) -> bool {
    let Some(text) = src.lines().nth(line.saturating_sub(1) as usize) else {
        return false;
    };
    let text = text.trim_start();
    let is_let = text.starts_with("if let ") || text.starts_with("while let ");
    if !is_let {
        return false;
    }
    let Some((header, _)) = text.split_once('=') else {
        return false;
    };
    header.contains('{') && header.contains('}')
}

/// `} else if let ...`: the lexer does not recognize `else if let`, so it merges
/// the condition header, the body statement, and the statement following the block
/// into a single expression event on the header's line. The CST records the condition
/// and each inner/outer statement on its own line.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_else_if_let_line(src: &str, line: u32) -> bool {
    let Some(text) = src.lines().nth(line.saturating_sub(1) as usize) else {
        return false;
    };
    let text = text.trim_start();
    text.contains("else if let ")
}

/// A call passing a closure with a block body: `on_big_stack(|| {`.
/// The one-pass lexer does not recognize closure blocks, so it merges the
/// closure header and every statement inside the block into a single expression
/// event on the call's line. The CST records the call and each inner statement
/// on its own line.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_closure_block_line(src: &str, line: u32) -> bool {
    let Some(text) = src.lines().nth(line.saturating_sub(1) as usize) else {
        return false;
    };
    let text = text.trim();
    text.contains("|| {") || (text.contains('|') && text.contains("| {"))
}

/// `pub fn lex(c: &mut Cursor)` and `pub struct Marker(u32)` reach the one-pass
/// lexer as calls: a name, then a parenthesis. The name is being declared, not
/// called, and the line says so — the declaration keyword is right before it.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_declaration_call(src: &str, name: &str, line: u32) -> bool {
    // An empty needle matches at every position without advancing, so the scan
    // below would never end. A call with no name is no name to look for.
    if name.is_empty() {
        return false;
    }
    const INTRODUCERS: [&str; 12] = [
        "fn", "func", "def", "struct", "enum", "trait", "type", "union", "class", "interface", "record", "module",
    ];
    let Some(text) = src.lines().nth(line.saturating_sub(1) as usize) else {
        return false;
    };
    let mut from = 0;
    while let Some(at) = text[from..].find(name) {
        let start = from + at;
        let prefix = text[..start].trim_end();
        if INTRODUCERS.iter().any(|word| prefix.ends_with(word)) {
            return true;
        }
        from = start + name.len();
    }
    false
}

/// One flag per line: is the line inside the brace body of a macro invocation
/// (`grammar! { … }`) or a macro definition (`macro_rules! name { … }`)? What a
/// macro receives is tokens, not code — the lexer reads `_name ("," _name)*`
/// inside a grammar as calls, and the CST records none because none is there.
#[cfg(all(feature = "deep", feature = "parser"))]
fn macro_body_lines(src: &str) -> Vec<bool> {
    let mut out = Vec::new();
    let mut stack: Vec<bool> = Vec::new();
    let mut inside = false;
    let mut tail = String::new();
    for ch in src.chars() {
        match ch {
            '\n' => {
                out.push(inside);
                tail.clear();
            }
            '{' | '(' | '[' => {
                let opener = tail.trim_end();
                // `grammar! { … }`, `matches!(…)`, `#[cfg(…)]`, and
                // `macro_rules! name { … }`: what these receive is tokens, not
                // code. The one-pass lexer reads every `name(` inside them as a
                // call, and the CST records none because none is there.
                let is_data = opener.ends_with('!')
                    || opener.ends_with('#')
                    || opener.contains("macro_rules!");
                stack.push(is_data);
                inside = inside || is_data;
                tail.clear();
            }
            '}' | ')' | ']' => {
                stack.pop();
                inside = stack.iter().any(|row| *row);
                tail.clear();
            }
            _ => {
                tail.push(ch);
                if tail.len() > 120 {
                    tail.remove(0);
                }
            }
        }
    }
    out.push(inside);
    out
}

#[cfg(all(feature = "deep", feature = "parser"))]
fn is_pattern_call(src: &str, name: &str, line: u32) -> bool {
    if name.is_empty() {
        return false;
    }
    let Some(text) = src.lines().nth(line.saturating_sub(1) as usize) else {
        return false;
    };
    let mut from = 0;
    while let Some(at) = text[from..].find(name) {
        let start = from + at + name.len();
        let rest = text[start..].trim_start();
        if !rest.starts_with('(') {
            from = start;
            continue;
        }
        let mut depth = 0i32;
        let mut end = None;
        for (index, ch) in rest.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(index);
                        break;
                    }
                }
                _ => {}
            }
        }
        if let Some(end) = end {
            let tail = rest[end + 1..].trim_start();
            // A match guard sits between the pattern and the arrow.
            if tail.starts_with("=>") || tail.contains("=>") && !tail.contains(';') {
                return true;
            }
            // `if let Some(x) = expr` / `while let`: the pattern sits between
            // `let` and the `=` that starts the value. A tuple or slice pattern
            // puts the bindings beside it and its own closing bracket in
            // between — `if let (Some(a), Some(b)) = pair` — so the structure
            // the pattern is nested in is stepped over before the `=` is read.
            let nested = tail.trim_start_matches(|ch: char| matches!(ch, ')' | ']' | ',') || ch.is_whitespace());
            if nested.starts_with('=') && !nested.starts_with("==") && text[..start].contains("let") {
                return true;
            }
        }
        from = start;
    }
    false
}

/// Whether a lexer assignment event names an imported type rather than a binder,
/// and the CST wrote the same statement with a value that covers it.

/// `var sentinelHandler Handler = noopHandler` — the one-pass lexer takes the
/// last name before `=` as the binder, so it binds the *type*. The statement is
/// in the CST's record with the real name and the same value, and the source
/// line shows both names in that order. Nothing is lost; one binder was
/// invented, and it is set aside here rather than forcing a fallback on every
/// Go file that declares a typed variable.
#[cfg(all(feature = "deep", feature = "parser"))]
fn type_as_target(src: &str, want: &(u8, Shape), found: &[(u8, Shape)]) -> bool {
    if want.0 != b'a' || src.is_empty() {
        return false;
    }
    let Some(line) = src.lines().nth(want.1.line.saturating_sub(1) as usize) else {
        return false;
    };
    let lexer_names = name_list(&want.1.targets);
    if lexer_names.is_empty() {
        return false;
    }
    found.iter().any(|row| {
        if row.0 != b'a' || (row.1.line as i64 - want.1.line as i64).abs() > 1 {
            return false;
        }
        if row.1.targets == want.1.targets || covers_value(&row.1, &want.1, src).is_err() {
            return false;
        }
        let deep_names = name_list(&row.1.targets);
        let Some(deep) = deep_names.first() else { return false };
        // `x: Dict[str, int] = ...` / `self.x: Dict[...] = ...`: the binder is
        // written first, the colon follows it, and the name the lexer bound is
        // the annotation's type. Both are on this line, in that order.
        let Some(colon) = line.find(':') else { return false };
        match (line.find(deep.as_str()), line.find(lexer_names[0].as_str())) {
            (Some(binder), Some(annotation)) => binder < colon && annotation > colon && line[annotation..].contains('='),
            _ => false,
        }
    })
}

/// The identifiers in a shape's text, quoted or bare: the lexer writes
/// `["x"]` and `[x]` for a binder depending on the language.
#[cfg(all(feature = "deep", feature = "parser"))]
fn name_list(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '.' {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}
#[cfg(all(feature = "deep", feature = "parser"))]
fn annotation_targets(lexer: &Facts, want: &(u8, Shape), found: &[(u8, Shape)], src: &str) -> bool {
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
            && (covers_value(&row.1, &want.1, src).is_ok() || row.1.value == want.1.value)
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

/// Two target lists name the same binders when they hold the same names, in any
/// order. `for opcode, arg, pos := range xs` reaches the two readers as the same
/// three names with different iteration order in one of them, and order carries
/// no meaning in a binder list — the names do.
#[cfg(all(feature = "deep", feature = "parser"))]
fn same_targets(deep: &str, lexer: &str) -> bool {
    if deep == lexer {
        return true;
    }
    let split = |text: &str| -> Vec<String> {
        // The lists are written with quoted names in some languages and bare
        // names in others (`["name","incoming"]`); the identifiers are the same
        // either way, and only they decide whether the binders match.
        let mut names = name_list(text);
        names.sort_unstable();
        names
    };
    let (left, right) = (split(deep), split(lexer));
    // A list that yields no names is not evidence of equality: the lexer writes
    // an unquoted binder (`["Dict"]`), and the atom reader only knows quoted
    // names, so an empty parse must never make two different lists equal.
    !left.is_empty() && left == right
}

#[cfg(all(feature = "deep", feature = "parser"))]
fn covers(deep: &Shape, lexer: &Shape, src: &str) -> Result<(), Miss> {
    // A lexer statement and the CST node it sits on can disagree by a line (a
    // signature stamped where it ends against where it starts), so an event is
    // paired near its line, never across the file: a duplicate target on
    // another line must not stand in for the event that lost its value.
    if (deep.line as i64 - lexer.line as i64).abs() > 1 {
        return Err(Miss::Line);
    }
    if !same_targets(&deep.targets, &lexer.targets) {
        return Err(Miss::Targets);
    }
    covers_value(deep, lexer, src)
}

#[cfg(all(feature = "deep", feature = "parser"))]
fn covers_value(deep: &Shape, lexer: &Shape, src: &str) -> Result<(), Miss> {
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
        // A path and its prefix name the same thing to a reader: the lexer
        // writes `rw` where the CST wrote `rw.Header` (or the reverse) when a
        // call sits in a receiver chain, and neither reading lost a fact.
        // A path and its prefix name the same thing to a reader (`rw` against
        // `rw.Header`), but that tolerance must not run before exactness: the
        // name `Class` would otherwise consume `Class.Node` and leave the real
        // `Class.Node` wants with nothing, which is how a pair of identical
        // values was reported as a missing name.
        let prefix_same = |row: &&String| {
            row.starts_with("n:") && name.starts_with("n:") && {
                let (row, name) = (&row[2..], &name[2..]);
                !row.is_empty() && !name.is_empty()
                    && (row.starts_with(&format!("{name}.")) || name.starts_with(&format!("{row}.")))
            }
        };
        match found
            .iter()
            .position(|row| row.as_str() == name.as_str())
            .or_else(|| found.iter().position(|row| prefix_same(&row)))
        {
            Some(at) => {
                found.swap_remove(at);
            }
            None => missing.push(name),
        }
    }
    if !missing.is_empty() {
        if std::env::var("RG_GUARD_DEBUG").is_ok() {
            // RG_GUARD_DEBUG=1 prints the pair, not a summary: reading a length
            // and guessing cost more than reading the values.
            eprintln!("MISS line {} deep=[{}] lex=[{}] missing={:?}", lexer.line, deep.value, lexer.value, missing);
        }
        let line_text = src.lines().nth(lexer.line.saturating_sub(1) as usize).unwrap_or("");
        if line_text.contains("b'") {
            missing.retain(|name| {
                let bare = name.strip_prefix("n:").unwrap_or(name);
                let is_byte_artefact = bare == "b"
                    || (bare.starts_with('x')
                        && bare.len() <= 4
                        && bare[1..].chars().all(|c| c.is_ascii_hexdigit()));
                !is_byte_artefact
            });
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
        let inner = rest.strip_suffix(')').unwrap_or(rest);
        // The receiver may itself be a call — `f(Path(__file__))` writes
        // `c(c(0;[n:__file__]);[])` — so the arguments start at the first
        // semicolon at depth zero, not at the first one in the string.
        let mut depth = 0i32;
        let mut cut = None;
        for (at, ch) in inner.char_indices() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ';' if depth == 0 => {
                    cut = Some(at);
                    break;
                }
                _ => {}
            }
        }
        let Some(split) = cut else {
            out.push("call".to_string());
            return;
        };
        let (receiver, arguments) = (&inner[..split], &inner[split + 1..]);
        let arguments = arguments.strip_prefix('[').unwrap_or(arguments);
        let arguments = arguments.strip_suffix(']').unwrap_or(arguments);
        let items = split_items(arguments);
        // A call with neither a receiver nor an argument names nothing: the
        // one-pass lexer writes one for a chain's own parentheses, and there is
        // no fact in it the CST can be asked to carry.
        if receiver == "0" && items.is_empty() {
            return;
        }
        out.push("call".to_string());
        // An empty receiver slot is the absence of a name, not a constant: the
        // lexer writes `c(0;…)` for a call it could not name a receiver for,
        // and counting that as a constant the CST must also carry made a
        // record that *names* the receiver read as a loss.
        if receiver != "0" {
            atoms(receiver, out);
        }
        for item in items {
            atoms(item, out);
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
fn shape_event(
    tag: u8,
    line: u32,
    payload: &str,
    calls: &[crate::facts::Call],
    accepted: &mut Accepted,
    src: &str,
    macro_lines: &[bool],
) -> Shape {
    if tag != b'a' {
        let mut pos = 0;
        let value = shape_at(payload, &mut pos, calls, accepted, src, macro_lines);
        return Shape { line, targets: String::new(), value: strip_literal_prefixes(value, src, line) };
    }
    let targets_end = element_end(payload.as_bytes(), 0);
    let targets = payload[..targets_end].to_string();
    let mut pos = targets_end;
    if payload.as_bytes().get(pos) == Some(&b',') {
        pos += 1;
    }
    let value = shape_at(payload, &mut pos, calls, accepted, src, macro_lines);
    Shape { line, targets, value: strip_literal_prefixes(value, src, line) }
}

/// `b' '`, `b"x"`, `r"..."`, `br#"..."#`, `c"..."`, `f"..."`: the one-pass
/// lexer reads the literal prefix as a name and the CST reads a single literal
/// token. A name that is exactly a prefix and sits in front of its quote on
/// that line names nothing, so neither path is asked to agree on it.
#[cfg(all(feature = "deep", feature = "parser"))]
fn strip_literal_prefixes(mut value: String, src: &str, line: u32) -> String {
    if !value.contains("n:") {
        return value;
    }
    // A statement can span several lines: `self.after_value = match kind {`
    // opens on one and the `b')'` that proves the prefix is a literal sits two
    // lines later. Look at the statement's neighbourhood, not its first line.
    let text: String = src
        .lines()
        .skip(line.saturating_sub(3) as usize)
        .take(5)
        .collect::<Vec<_>>()
        .join("\n");
    for prefix in ["br", "b", "r", "c", "f"] {
        if !text.contains(&format!("{prefix}'")) && !text.contains(&format!("{prefix}\"")) {
            continue;
        }
        // The shaped form writes a name as `n:name`, and a name is an element
        // of a list: it stands between a `[` or `,` and a `,` or `]`.
        let atom = format!("n:{prefix}");
        loop {
            let mut found = None;
            let mut from = 0;
            while let Some(at) = value[from..].find(&atom) {
                let at = from + at;
                let end = at + atom.len();
                let before = if at == 0 { b'[' } else { value.as_bytes()[at - 1] };
                let after = value.as_bytes().get(end).copied().unwrap_or(b']');
                let opens = before == b'[' || before == b',' || before == b':';
                let closes = after == b',' || after == b']';
                if opens && closes {
                    found = Some((at, end, before, after));
                    break;
                }
                from = end;
            }
            let Some((at, end, before, after)) = found else { break };
            if before == b',' {
                value.replace_range(at - 1..end, "");
            } else if after == b',' {
                value.replace_range(at..end + 1, "");
            } else {
                value.replace_range(at..end, "");
            }
            while value.contains("[[]]") {
                value = value.replace("[[]]", "[]");
            }
        }
    }
    value
}

/// The form of one value at `pos`, with call numbering erased: a call
/// contributes its receiver and arguments, never its symbol-local index. An
/// anonymous call (`f(x)(y)` is named `.call`) contributes the value it applies;
/// the application has no name for the CST to read, so it is not a fact either
/// path can be asked to agree on.
#[cfg(all(feature = "deep", feature = "parser"))]
fn shape_at(
    text: &str,
    pos: &mut usize,
    calls: &[crate::facts::Call],
    accepted: &mut Accepted,
    src: &str,
    macro_lines: &[bool],
) -> String {
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
            let receiver = shape_at(inner, &mut at, calls, accepted, src, macro_lines);
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
                        let arg = shape_at(list, &mut item, calls, accepted, src, macro_lines);
                        if !arg.is_empty() {
                            arguments.push(arg);
                        }
                    } else {
                        item += 1;
                    }
                }
            }
            let call = calls.get(id);
            let path = call.map(|call| call.path.as_str()).unwrap_or("");
            if phantom_call(path) == Some(Phantom::Anonymous) {
                return receiver;
            }
            if phantom_call(path) == Some(Phantom::Keyword) {
                let name = call.map(|call| call.path.as_str()).unwrap_or("");
                return if arguments.is_empty() {
                    format!("n:{name}")
                } else {
                    format!("m:[n:{name},{}]", arguments.join(","))
                };
            }
            if let Some(call) = call {
                let name = call.path.rsplit('.').next().unwrap_or(&call.path);
                if !src.is_empty() && is_positional_call(src, macro_lines, name, call.line) {
                    return String::new();
                }
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
                        let item = shape_at(inner, &mut at, calls, accepted, src, macro_lines);
                        if !item.is_empty() {
                            items.push(item);
                        }
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
            format!("k({name}):{}", shape_at(inner, &mut at, calls, accepted, src, macro_lines))
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
    // `--deep` may force a language whose record `auto` would not choose; the
    // eligibility list differs, the parse does not.
    let requested = if mode == PathMode::Deep { deep_facts::lang_for_forced } else { deep_facts::lang_for };
    let lang = match requested(language.tag) {
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
        // The lexer is the fast path, not the only one: if it found nothing and
        // the CST found a record, the CST is the record. This is the fall
        // *forward* — a one-pass reader that failed cannot be trusted to say
        // what a file contains, and the deeper reader can.
        if lexer.symbols.is_empty() && !deep.symbols.is_empty() {
            deep_run::record(
                path,
                deep_run::Taken {
                    deep: true,
                    compared: false,
                    errors: cst.errors,
                    fallback: Some("the one-pass lexer found no symbols".to_string()),
                    why: "lexer-empty",
                    ..deep_run::Taken::default()
                },
            );
            return deep;
        }
        let verdict = loses_in(&lexer, &deep, &String::from_utf8_lossy(bytes));
        if verdict.lost() {
            // Name the fact, not just the fact that there was one: a reader
            // looking at a fallback must be able to see what the CST missed
            // without re-running both paths by hand.
            let fallback = Some(match verdict.why() {
                Some(why) => format!("the CST lost a {why}: {}", verdict.detail()),
                None => "the CST record lost facts the lexer found".to_string(),
            });
            deep_run::record(
                path,
                deep_run::Taken { compared: true, fallback, accepted: verdict.accepted(), ..deep_run::Taken::default() },
            );
            return lexer;
        }
        let mut lexer_weight = weight(&lexer);
        lexer_weight[0] = lexer_weight[0].saturating_sub(
            (verdict.accepted().invented_symbols + verdict.accepted().anonymous_symbols) as u64,
        );
        let why = match weight(&deep).cmp(&lexer_weight) {
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

/// Does the line the lexer says a definition starts on actually declare it?
///
/// The one-pass reader opens a symbol whenever a name is followed by `(` inside
/// a statement it does not model — `for _, x := range []string{…}` yields a
/// definition named `string`, `if err := f(); err != nil` one named `err`. Each
/// language spells a definition with a keyword, so the source is the appeal: no
/// keyword, no definition.
/// Keywords that can never name a callee, per lexer tag. A call with one of
/// these names is the one-pass reader's reconstruction of a statement it does
/// not model, not a fact about the source.
#[cfg(all(feature = "deep", feature = "parser"))]
fn is_keyword_callee(tag: &str, name: &str) -> bool {
    let keywords: &[&str] = match tag {
        "PY" => &["assert", "del", "raise", "with", "lambda", "return", "yield", "await", "not"],
        "JS" | "TS" => &["function", "if", "for", "while", "return", "typeof", "delete", "await", "yield", "switch", "catch"],
        "RS" => &["fn", "if", "for", "while", "return", "match", "loop", "let", "unsafe"],
        "GO" => &["func", "go", "defer", "range", "select", "switch", "if", "for", "type", "var", "const", "return"],
        "JAVA" => &[
            "new", "if", "while", "for", "switch", "return", "throw", "synchronized", "assert", "try", "catch", "instanceof",
        ],
        _ => &["if", "for", "while", "return", "switch", "function", "func", "fn", "def"],
    };
    keywords.contains(&name)
}

#[cfg(all(feature = "deep", feature = "parser"))]
fn claims_the_line(src: &str, tag: &str, line: u32, name: &str) -> bool {
    let Some(text) = src.lines().nth(line.saturating_sub(1) as usize) else {
        return false;
    };
    if !text.contains(name) {
        return false;
    }
    let keywords: &[&str] = match tag {
        "PY" => &["def ", "class "],
        "JS" | "TS" => &["function ", "class ", "const ", "let ", "var ", "=> "],
        "RS" => &["fn ", "struct ", "enum ", "trait ", "impl ", "mod "],
        "GO" => &["func ", "type ", "var ", "const "],
        "JAVA" => &["class ", "interface ", "enum ", "record ", "void ", "public ", "private ", "protected "],
        _ => &["def ", "function ", "func ", "fn ", "class ", "struct "],
    };
    // The name has to be introduced by the keyword, not merely share the line
    // with one: on `pub(crate) struct Meter {` the lexer files a symbol named
    // `pub`, and the line does declare something — just not that.
    let declared = keywords.iter().any(|keyword| {
        text.match_indices(keyword).any(|(at, _)| match text[at + keyword.len()..].find(name) {
            Some(gap) => gap <= 24,
            None => false,
        })
    });
    // `exports.three = (z) => z`: a name bound by an assignment to a member is
    // a definition the lexer names and the tree must keep, and it is written
    // before the `=`, not after a keyword.
    let assigned = text.match_indices(name).any(|(at, _)| {
        text[at + name.len()..].trim_start().starts_with('=') || text[at + name.len()..].trim_start().starts_with(":=")
    });
    declared || assigned
}
