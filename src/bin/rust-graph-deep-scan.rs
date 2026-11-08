//! The separate deep-scan binary, built only with `--features parser`.
//!
//! ```text
//! rust-graph-deep-scan --version
//! rust-graph-deep-scan tree  FILE            the tree as JSON on stdout
//! rust-graph-deep-scan sexp  FILE [--all]    the tree as an S-expression
//! rust-graph-deep-scan probe FILE [--reps N] [--max-depth D] [--stack-kb K]
//! rust-graph-deep-scan bench LIST [--reps N]  files named one per line in LIST
//! rust-graph-deep-scan compare LIST [--reps N]  JSON: the grammar! parser against the hand one
//! ```
//! `bench` mirrors the Tree-sitter driver used in Phase 1: every file is read
//! first, then each is parsed `--reps` times and the best time counts.
//! `probe` prints one line of measurements. `--stack-kb` parses on a thread with
//! exactly that stack, to measure stack use per nesting level. `compare` fails
//! unless both JSON parsers give the same events, tree and outcome for every
//! file, and reports both speeds, timed alternately on the same bytes.

#[path = "../deep/mod.rs"]
mod deep;

use std::process::ExitCode;
use std::time::Instant;

use deep::syntax::Language;
use deep::tree::{Event, Node, HAS_ERROR};
use deep::{Budget, Workspace};

/// Parses run on their own thread with this much stack unless `--stack-kb` says
/// otherwise. Only touched pages cost memory.
const PARSE_STACK: usize = 64 << 20;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("rust-graph-deep-scan: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let (command, rest) = args.split_first().ok_or_else(usage)?;
    if command == "--version" {
        println!("rust-graph-deep-scan {} (parser {})", env!("CARGO_PKG_VERSION"), deep::PARSER_VERSION);
        return Ok(());
    }
    let (path, options) = rest.split_first().ok_or_else(usage)?;
    if command == "bench" || command == "compare" {
        let is_bench = command == "bench";
        let reps = match options {
            [flag, value] if flag == "--reps" => value.parse().map_err(|_| "--reps needs a number".to_string())?,
            [] => 5,
            _ => return Err(usage()),
        };
        let list = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
        let files: Vec<String> = list.lines().filter(|line| !line.is_empty()).map(str::to_string).collect();
        return std::thread::Builder::new()
            .stack_size(PARSE_STACK)
            .spawn(move || if is_bench { bench(&files, reps) } else { compare(&files, reps) })
            .map_err(|error| format!("cannot start the parse thread: {error}"))?
            .join()
            .map_err(|_| "the parse thread panicked".to_string())?;
    }
    let language = deep::language_for(path).ok_or_else(|| format!("{path}: no grammar for this file type yet (Slice 1 has JSON)"))?;
    let src = std::fs::read(path).map_err(|error| format!("{path}: {error}"))?;
    let mut budget = Budget::default();
    let mut reps = 1usize;
    let mut stack = PARSE_STACK;
    let mut all = false;
    let mut at = 0;
    while at < options.len() {
        let value = || options.get(at + 1).and_then(|value| value.parse::<u64>().ok()).ok_or_else(|| format!("{} needs a number", options[at]));
        match options[at].as_str() {
            "--reps" => reps = value()?.max(1) as usize,
            "--max-depth" => budget.max_depth = value()? as u32,
            "--stack-kb" => stack = value()? as usize * 1024,
            "--all" => {
                all = true;
                at += 1;
                continue;
            }
            other => return Err(format!("unknown option {other}")),
        }
        at += 2;
    }
    let command = command.clone();
    let path = path.clone();
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || execute(&command, &path, &src, language, &budget, reps, all))
        .map_err(|error| format!("cannot start the parse thread: {error}"))?
        .join()
        .map_err(|_| "the parse thread panicked".to_string())?
}

fn execute(command: &str, path: &str, src: &[u8], language: &'static Language, budget: &Budget, reps: usize, all: bool) -> Result<(), String> {
    let mut workspace = Workspace::default();
    let mut best = f64::INFINITY;
    let mut first = 0.0;
    let mut outcome = None;
    for rep in 0..reps {
        let start = Instant::now();
        outcome = Some(workspace.parse(src, language, budget).map_err(|_| format!("{path}: over 4 GiB"))?);
        let elapsed = start.elapsed().as_secs_f64() * 1e3;
        if rep == 0 {
            first = elapsed;
        }
        best = best.min(elapsed);
    }
    let outcome = outcome.expect("reps is at least 1");
    let tree = &workspace.tree;
    match command {
        "tree" => {
            let mut out = String::with_capacity(tree.len() * 64);
            tree.write_json(language, src, &mut out);
            println!("{out}");
            eprintln!("{}", outcome.describe());
        }
        "sexp" => {
            println!("root={}", language.name(tree.root().kind));
            println!("{}", tree.sexp(language, !all));
        }
        "probe" => {
            let walk = Instant::now();
            let depth = tree.depth();
            let walk_ms = walk.elapsed().as_secs_f64() * 1e3;
            let errors = tree.nodes.iter().filter(|node| node.kind == deep::syntax::ERROR).count();
            let arena = workspace.tokens.kind.capacity() * 2
                + workspace.tokens.end.capacity() * 4
                + workspace.pairs.pair.capacity() * 4
                + workspace.events.capacity() * std::mem::size_of::<Event>()
                + tree.nodes.capacity() * std::mem::size_of::<Node>();
            println!(
                "bytes={} root={} clean={} first_ms={:.2} parse_ms={:.2} MB_s={:.1} walk_ms={:.2} nodes={} depth={} error_nodes={} arena_bytes={} arena_per_byte={:.2} node_bytes={} event_bytes={} rss_kb={} {}",
                src.len(),
                language.name(tree.root().kind),
                tree.root().flags & HAS_ERROR == 0,
                first,
                best,
                src.len() as f64 / 1e6 / (best / 1e3),
                walk_ms,
                tree.len(),
                depth,
                errors,
                arena,
                arena as f64 / src.len().max(1) as f64,
                std::mem::size_of::<Node>(),
                std::mem::size_of::<Event>(),
                peak_rss_kb(),
                outcome.describe(),
            );
        }
        other => return Err(format!("unknown command {other}\n{}", usage())),
    }
    Ok(())
}

fn bench(files: &[String], reps: usize) -> Result<(), String> {
    let mut inputs = Vec::new();
    for path in files {
        let language = deep::language_for(path).ok_or_else(|| format!("{path}: no grammar for this file type yet"))?;
        inputs.push((std::fs::read(path).map_err(|error| format!("{path}: {error}"))?, language));
    }
    let budget = Budget::default();
    let mut workspace = Workspace::default();
    let (mut best_total, mut first_total) = (0.0, 0.0);
    let (mut bytes, mut clean, mut nodes, mut error_bytes, mut deepest, mut incomplete) = (0usize, 0usize, 0usize, 0u64, 0u32, 0usize);
    for (src, language) in &inputs {
        let mut best = f64::INFINITY;
        for rep in 0..reps.max(1) {
            let start = Instant::now();
            let outcome = workspace.parse(src, language, &budget).map_err(|_| "a file over 4 GiB".to_string())?;
            let elapsed = start.elapsed().as_secs_f64() * 1e3;
            if rep == 0 {
                first_total += elapsed;
                if !outcome.complete() {
                    incomplete += 1;
                }
            }
            best = best.min(elapsed);
        }
        best_total += best;
        bytes += src.len();
        let tree = &workspace.tree;
        nodes += tree.len();
        deepest = deepest.max(tree.depth());
        if tree.root().flags & HAS_ERROR == 0 {
            clean += 1;
        }
        // Bytes under outermost ERROR nodes, as the Tree-sitter driver counts them.
        let mut at = 0;
        while at < tree.len() {
            let node = tree.nodes[at];
            if node.kind == deep::syntax::ERROR {
                error_bytes += u64::from(node.end - node.start);
                at = node.subtree_end as usize;
            } else {
                at += 1;
            }
        }
    }
    println!(
        "files={} bytes={} best_ms={:.1} first_pass_ms={:.1} MB_s={:.1} clean_files={}/{} err_bytes={} ({:.3}%) nodes={} node_per_byte={:.3} maxdepth={} incomplete={} rss_kb={}",
        inputs.len(),
        bytes,
        best_total,
        first_total,
        bytes as f64 / 1e6 / (best_total / 1e3),
        clean,
        inputs.len(),
        error_bytes,
        100.0 * error_bytes as f64 / bytes.max(1) as f64,
        nodes,
        nodes as f64 / bytes.max(1) as f64,
        deepest,
        incomplete,
        peak_rss_kb(),
    );
    Ok(())
}

fn compare(files: &[String], reps: usize) -> Result<(), String> {
    let mut inputs = Vec::new();
    for path in files {
        if !path.to_ascii_lowercase().ends_with(".json") {
            return Err(format!("{path}: compare is for JSON, the one language with both parsers"));
        }
        inputs.push(std::fs::read(path).map_err(|error| format!("{path}: {error}"))?);
    }
    let budget = Budget::default();
    let (mut hand, mut generated) = (Workspace::default(), Workspace::default());
    let (mut bytes, mut identical, mut hand_ms, mut macro_ms) = (0usize, 0usize, 0.0, 0.0);
    for (path, src) in files.iter().zip(&inputs) {
        let want = hand.parse(src, &deep::json::JSON, &budget).map_err(|_| format!("{path}: over 4 GiB"))?;
        let got = generated.parse(src, &deep::json_grammar::JSON, &budget).map_err(|_| format!("{path}: over 4 GiB"))?;
        if got == want && generated.events == hand.events && generated.tree.nodes == hand.tree.nodes {
            identical += 1;
        } else {
            eprintln!("{path}: the parsers differ");
        }
        let (mut hand_best, mut macro_best) = (f64::INFINITY, f64::INFINITY);
        for _ in 0..reps.max(1) {
            let start = Instant::now();
            hand.parse(src, &deep::json::JSON, &budget).map_err(|_| format!("{path}: over 4 GiB"))?;
            hand_best = hand_best.min(start.elapsed().as_secs_f64() * 1e3);
            let start = Instant::now();
            generated.parse(src, &deep::json_grammar::JSON, &budget).map_err(|_| format!("{path}: over 4 GiB"))?;
            macro_best = macro_best.min(start.elapsed().as_secs_f64() * 1e3);
        }
        hand_ms += hand_best;
        macro_ms += macro_best;
        bytes += src.len();
    }
    let rate = |ms: f64| bytes as f64 / 1e6 / (ms / 1e3);
    println!(
        "files={} bytes={} identical={}/{} hand_ms={:.1} macro_ms={:.1} hand_MB_s={:.1} macro_MB_s={:.1} macro_vs_hand={:+.1}%",
        inputs.len(),
        bytes,
        identical,
        inputs.len(),
        hand_ms,
        macro_ms,
        rate(hand_ms),
        rate(macro_ms),
        100.0 * (rate(macro_ms) / rate(hand_ms) - 1.0),
    );
    if identical == inputs.len() {
        Ok(())
    } else {
        Err(format!("{} of {} files differ", inputs.len() - identical, inputs.len()))
    }
}

fn peak_rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| status.lines().find(|line| line.starts_with("VmHWM:")).and_then(|line| line.split_whitespace().nth(1)?.parse().ok()))
        .unwrap_or(0)
}

fn usage() -> String {
    "usage: rust-graph-deep-scan --version | tree FILE | sexp FILE [--all] | probe FILE [--reps N] [--max-depth D] [--stack-kb K] | bench LIST [--reps N] | compare LIST [--reps N]".into()
}
