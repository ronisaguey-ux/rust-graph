//! What the lexer costs: tokens per second on a 1 MB file, allocations per
//! token, and peak RSS.
//!
//! ```text
//! cargo run --release -p deep-lex --example measure
//! ```
//!
//! A counting `GlobalAlloc` wraps the system allocator for the whole run, so
//! the allocation numbers are measured, not asserted. The input is generated
//! here rather than read from disk: the same bytes every run, no fixture drift.
//!
//! Peak RSS comes from `/proc/self/status` (`VmHWM`), which is the high-water
//! mark since process start, so the before/after pair brackets the whole
//! build-and-lex phase.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use deep_lex::{DeepLexer, Kind, Language, Lexer};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size, Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        System.alloc_zeroed(layout)
    }
}

#[global_allocator]
static COUNTER: Counting = Counting;

fn peak_rss_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            return rest
                .split_whitespace()
                .next()
                .and_then(|kb| kb.parse().ok())
                .unwrap_or(0);
        }
    }
    0
}

fn rss_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest
                .split_whitespace()
                .next()
                .and_then(|kb| kb.parse().ok())
                .unwrap_or(0);
        }
    }
    0
}

/// Grow `unit` until the buffer is at least 1 MB.
fn repeat_to_1mb(unit: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 << 20);
    while out.len() < 1 << 20 {
        out.extend_from_slice(unit);
    }
    out.truncate(1 << 20);
    out
}

fn python_mb() -> Vec<u8> {
    repeat_to_1mb(
        b"import os\n\nclass Point:\n    def __init__(self, x, y=0.5):\n        self.x = x  # keep\n        self.y = \"y\\n\"\n\n    def norm(self):\n        return (self.x ** 2 + self.y ** 2) ** 0.5\n\nfor i in range(100):\n    if i % 2 == 0:\n        print(i, end='')\n    else:\n        print(`i`)\n",
    )
}

fn javascript_mb() -> Vec<u8> {
    repeat_to_1mb(
        b"const re = /ab+c/gi;\nfunction add(a, b = 2, ...rest) {\n  return a + b;\n}\nclass Point { constructor(x) { this.x = x; } }\nconst s = `a${b}c`;\nif (re.test(s)) { x = a / b; }\nlet n = 0xFF + 1_000 + 3.5e-2;\n",
    )
}

fn json_mb() -> Vec<u8> {
    repeat_to_1mb(
        b"{\"name\": \"value\", \"n\": -12, \"xs\": [1, 2.5, 3e4], \"t\": true, \"f\": false, \"z\": null}\n",
    )
}

fn measure(label: &str, src: &[u8], lang: Language) {
    // Warm up once so the bracket Vec's capacity growth, if any, is not counted
    // as per-token cost; then measure a fresh lexer on the same bytes.
    {
        let mut warm = DeepLexer::new(src, lang);
        while warm.next_token(0).is_some() {}
    }
    let rss_before = rss_kb();
    let allocs_before = ALLOCS.load(Ordering::Relaxed);
    let bytes_before = BYTES.load(Ordering::Relaxed);

    // Construction separately: the bracket pass is one O(n) scan and its
    // `pair` Vec is the lexer's only allocation.
    let build_start = Instant::now();
    let mut lexer = DeepLexer::new(src, lang);
    let build = build_start.elapsed();

    let start = Instant::now();
    let mut tokens = 0u64;
    let mut significant = 0u64;
    while let Some(token) = lexer.next_token(0) {
        tokens += 1;
        // Compare symbols directly: `Kind::from_symbol` is a linear search and
        // would measure itself, not the lexer.
        let layout = token.symbol == Kind::Newline as u16
            || token.symbol == Kind::Indent as u16
            || token.symbol == Kind::Dedent as u16;
        if !layout {
            significant += 1;
        }
    }
    let elapsed = start.elapsed();

    let allocs = ALLOCS.load(Ordering::Relaxed) - allocs_before;
    let bytes = BYTES.load(Ordering::Relaxed) - bytes_before;
    let rss_after = rss_kb();
    let peak = peak_rss_kb();

    let seconds = elapsed.as_secs_f64();
    let per_second = tokens as f64 / seconds;
    let mb_per_second = (src.len() as f64 / (1 << 20) as f64) / seconds;
    let per_token = allocs as f64 / tokens as f64;

    println!("{label}");
    println!("  input            {} bytes ({} KiB)", src.len(), src.len() / 1024);
    println!("  tokens           {tokens} ({significant} significant, {} layout)", tokens - significant);
    println!("  construction     {:.3} ms (bracket pair pass)", build.as_secs_f64() * 1e3);
    println!("  lexing           {:.3} ms", seconds * 1e3);
    println!("  throughput       {per_second:.0} tokens/s, {mb_per_second:.1} MiB/s");
    println!("  ns per token     {:.1}", seconds * 1e9 / tokens as f64);
    println!("  allocations      {allocs} ({bytes} bytes), {per_token:.6} per token");
    println!("  bracket pairs    {} bytes of pair[]", lexer.pair().len() * 4);
    println!("  RSS before/after {rss_before} / {rss_after} KiB, peak {peak} KiB");
    println!();

    // The claim: one allocation for the bracket table, none per token.
    assert!(
        allocs <= 1,
        "{label}: {allocs} allocations for {tokens} tokens"
    );
    assert!(per_token < 1e-5, "{label}: {per_token} allocations per token");
}

fn main() {
    let baseline = peak_rss_kb();
    println!("deep-lex measurements (release build recommended)");
    println!("baseline peak RSS {baseline} KiB\n");

    measure("python (1 MiB)", &python_mb(), Language::Python);
    measure("javascript (1 MiB)", &javascript_mb(), Language::JavaScript);
    measure("json (1 MiB)", &json_mb(), Language::Json);

    // A bracket-free input must not allocate at all, not even the pair Vec.
    let plain = repeat_to_1mb(b"identifier 42 another_name 7\n");
    let before = ALLOCS.load(Ordering::Relaxed);
    let mut lexer = DeepLexer::new(&plain, Language::Generic);
    let mut tokens = 0u64;
    while lexer.next_token(0).is_some() {
        tokens += 1;
    }
    let allocs = ALLOCS.load(Ordering::Relaxed) - before;
    println!("bracket-free 1 MiB: {tokens} tokens, {allocs} allocations");
    assert_eq!(allocs, 0, "a bracket-free file allocates nothing");
    println!("final peak RSS {} KiB", peak_rss_kb());
}
