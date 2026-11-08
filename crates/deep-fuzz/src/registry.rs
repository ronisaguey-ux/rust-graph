//! Which languages the fuzzer knows, and where their seed fixtures live.
//!
//! The list is explicit. A language without an entry is not fuzzed, and a
//! fixture directory named by an entry but missing from disk is an error the
//! caller sees, never a silent skip. The grammar statics are the same ones the
//! `deep-grammars` tests and the deep-scan binary use.

use std::path::{Path, PathBuf};

use deep_grammars::batch_a::{csharp::CSHARP, go::GO, java::JAVA, kotlin::KOTLIN};
use deep_grammars::batch_b::{php::PHP, ruby::RUBY, scala::SCALA, swift::SWIFT};
use deep_grammars::batch_c::{c::C, cpp::CPP, objc::OBJC};
use deep_grammars::batch_d::{
    awk::AWK, clojure::CLOJURE, css::CSS, elixir::ELIXIR, erlang::ERLANG, html::HTML, json::JSON, julia::JULIA, lua::LUA, perl::PERL, r::R,
    shell::SHELL, solidity::SOLIDITY, sql::SQL, tcl::TCL, yaml::YAML, zig::ZIG,
};
use deep_grammars::deep::syntax::Language;
use deep_grammars::javascript::{JAVASCRIPT, TYPESCRIPT};
use deep_grammars::python::PYTHON;

/// One fuzzable language: its grammar static, the fixture directories under
/// `crates/deep-grammars/fixtures`, and the extension to pick files with.
pub struct Entry {
    pub name: &'static str,
    pub language: &'static Language,
    /// Directories relative to the fixtures root, in the order seeds are taken.
    pub dirs: &'static [&'static str],
    /// Only files ending in `.ext` are seeds.
    pub ext: &'static str,
}

macro_rules! entry {
    ($name:literal, $lang:expr, $ext:literal, [$($dir:literal),*]) => {
        Entry { name: $name, language: $lang, dirs: &[$($dir),*], ext: $ext }
    };
}

/// Every language with fixtures in this worktree, sorted by name.
pub const ENTRIES: &[Entry] = &[
    entry!("awk", &AWK, "awk", ["longtail/awk", "longtail/malformed"]),
    entry!("c", &C, "c", ["c"]),
    entry!("clojure", &CLOJURE, "clj", ["longtail/clojure", "longtail/malformed"]),
    entry!("cpp", &CPP, "cpp", ["cpp"]),
    entry!("csharp", &CSHARP, "cs", ["csharp"]),
    entry!("css", &CSS, "css", ["longtail/css", "longtail/malformed"]),
    entry!("elixir", &ELIXIR, "ex", ["longtail/elixir", "longtail/malformed"]),
    entry!("erlang", &ERLANG, "erl", ["longtail/erlang", "longtail/malformed"]),
    entry!("go", &GO, "go", ["go"]),
    entry!("html", &HTML, "html", ["longtail/html", "longtail/malformed"]),
    entry!("java", &JAVA, "java", ["java"]),
    entry!("javascript", &JAVASCRIPT, "js", ["javascript", "malformed"]),
    entry!("json", &JSON, "json", ["longtail/json", "longtail/malformed"]),
    entry!("julia", &JULIA, "jl", ["longtail/julia", "longtail/malformed"]),
    entry!("kotlin", &KOTLIN, "kt", ["kotlin"]),
    entry!("lua", &LUA, "lua", ["longtail/lua", "longtail/malformed"]),
    entry!("objc", &OBJC, "m", ["objc"]),
    entry!("perl", &PERL, "pl", ["longtail/perl", "longtail/malformed"]),
    entry!("php", &PHP, "php", ["php"]),
    entry!("python", &PYTHON, "py", ["python", "malformed"]),
    entry!("r", &R, "R", ["longtail/r", "longtail/malformed"]),
    entry!("ruby", &RUBY, "rb", ["ruby"]),
    entry!("scala", &SCALA, "scala", ["scala"]),
    entry!("shell", &SHELL, "sh", ["longtail/shell", "longtail/malformed"]),
    entry!("solidity", &SOLIDITY, "sol", ["longtail/solidity", "longtail/malformed"]),
    entry!("sql", &SQL, "sql", ["longtail/sql", "longtail/malformed"]),
    entry!("swift", &SWIFT, "swift", ["swift"]),
    entry!("tcl", &TCL, "tcl", ["longtail/tcl", "longtail/malformed"]),
    entry!("typescript", &TYPESCRIPT, "ts", ["typescript", "malformed"]),
    entry!("yaml", &YAML, "yaml", ["longtail/yaml", "longtail/malformed"]),
    entry!("zig", &ZIG, "zig", ["longtail/zig", "longtail/malformed"]),
];

/// The `crates/deep-grammars/fixtures` directory inside this worktree.
///
/// Compile-time location: the fuzzer is a repo tool, and a binary copied
/// elsewhere needs `--fixtures` (see `main.rs`) because the fixtures are data,
/// not part of the binary.
pub fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../deep-grammars/fixtures")
}

/// The fuzzer's own hostile seeds, shared by every language.
pub fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/seeds")
}

pub fn by_name(name: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|entry| entry.name.eq_ignore_ascii_case(name))
}

/// The language for a file extension, case-insensitively.
pub fn by_extension(extension: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|entry| entry.ext.eq_ignore_ascii_case(extension))
}

/// Every fixture Seed for one language, plus the shared hostile seeds.
///
/// Seeds are capped at `MAX_SEED_BYTES` and `MAX_SEEDS` so a fuzz second is
/// spent mutating, not re-reading a large file. Selection is deterministic: the
/// files are sorted, then spread evenly across the cap so both a small file and
/// a large one are represented.
pub fn seeds_for(entry: &Entry) -> Result<Vec<Seed>, String> {
    seeds_in(&fixtures_root(), entry)
}

/// The size past which a fixture is not used as a seed.
pub const MAX_SEED_BYTES: usize = 32 * 1024;
/// The number of language fixtures kept, plus every hostile seed.
pub const MAX_SEEDS: usize = 8;

pub struct Seed {
    pub origin: String,
    pub bytes: Vec<u8>,
}

pub fn seeds_in(root: &Path, entry: &Entry) -> Result<Vec<Seed>, String> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for directory in entry.dirs {
        let dir = root.join(directory);
        if !dir.is_dir() {
            return Err(format!("{}: fixture directory {dir:?} does not exist", entry.name));
        }
        let mut found = Vec::new();
        walk(&dir, &mut found)?;
        found.retain(|path| path.extension().map(|ext| ext.eq_ignore_ascii_case(entry.ext)).unwrap_or(false));
        found.sort();
        paths.extend(found);
    }
    // Even spread over the (sorted) fixture list: first, last and the middle
    // files, which for the grammar crates cover valid and malformed inputs.
    let mut picked: Vec<PathBuf> = Vec::new();
    if paths.len() <= MAX_SEEDS {
        picked = paths;
    } else {
        for at in 0..MAX_SEEDS {
            picked.push(paths[at * (paths.len() - 1) / (MAX_SEEDS - 1)].clone());
        }
        picked.dedup();
    }
    let mut seeds = Vec::new();
    for path in picked {
        let bytes = std::fs::read(&path).map_err(|error| format!("{path:?}: {error}"))?;
        if bytes.len() <= MAX_SEED_BYTES {
            seeds.push(Seed { origin: path.display().to_string(), bytes });
        }
    }
    let hostile = corpus_root();
    if hostile.is_dir() {
        let mut found = Vec::new();
        walk(&hostile, &mut found)?;
        found.sort();
        for path in found {
            let bytes = std::fs::read(&path).map_err(|error| format!("{path:?}: {error}"))?;
            if bytes.len() <= MAX_SEED_BYTES {
                seeds.push(Seed { origin: path.display().to_string(), bytes });
            }
        }
    }
    if seeds.is_empty() {
        return Err(format!("{}: no seed fixtures under {root:?}", entry.name));
    }
    Ok(seeds)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|error| format!("{dir:?}: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("{dir:?}: {error}"))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|error| format!("{path:?}: {error}"))?;
        if kind.is_dir() {
            walk(&path, out)?;
        } else if kind.is_file() {
            out.push(path);
        }
    }
    Ok(())
}
