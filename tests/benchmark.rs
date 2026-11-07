//! The performance targets, as a test. Timing a debug build proves nothing, so
//! this only runs against an optimised binary:
//!
//!     cargo test --release --target x86_64-unknown-linux-musl --test benchmark -- --nocapture
#![cfg(not(debug_assertions))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_rust-graph");
const MAX_BINARY_BYTES: u64 = 2 * 1024 * 1024;
const MAX_COLD_START: Duration = Duration::from_millis(50);
const MAX_THOUSAND_FILES: Duration = Duration::from_millis(300);
const MAX_PEAK_RSS_KB: u64 = 64 * 1024;
const RUNS: usize = 7;

fn python_file(index: usize) -> String {
    let mut body = format!("import os\nfrom pkg{}.mod{} import helper_{}\n\n\n", index % 10, (index + 1) % 100, (index + 1) % 100);
    for function in 0..12 {
        body.push_str(&format!(
            "def work_{index}_{function}(value, flag=None):\n    \"\"\"Do step {function}.\"\"\"\n    name = f\"{{value}}-{function}\"\n    if flag:\n        name = name.strip()\n    else:\n        name = helper_{}(name)\n    total = len(name) + {function}\n    return os.path.join(name, str(total))\n\n\n",
            (index + 1) % 100
        ));
    }
    body.push_str(&format!("def helper_{}(value):\n    return value\n", index % 100));
    body
}

fn js_file(index: usize) -> String {
    let mut body = format!("const dep = require('./f{}');\n\n", (index + 1) % 100);
    for function in 0..12 {
        body.push_str(&format!(
            "function work{index}x{function}(value) {{\n  const text = `item ${{value}} {function}`;\n  if (!text) {{\n    return null;\n  }}\n  return dep.helper(text) + value.length;\n}}\n\n",
        ));
    }
    body.push_str("function helper(v) {\n  return v;\n}\nmodule.exports = { helper };\n");
    body
}

fn c_file(index: usize) -> String {
    let mut body = String::from("#include <stdio.h>\n#include \"util.h\"\n\n");
    for function in 0..12 {
        body.push_str(&format!(
            "static int work_{index}_{function}(const char *value, int size) {{\n  char buffer[64];\n  int total = size + {function};\n  if (value == NULL) {{\n    return -1;\n  }}\n  snprintf(buffer, sizeof buffer, \"%s\", value);\n  return total + helper_{}(buffer);\n}}\n\n",
            index % 50
        ));
    }
    body.push_str(&format!("int helper_{index}(const char *v) {{\n  return v != NULL;\n}}\n"));
    body
}

fn tree(name: &str, files: usize) -> PathBuf {
    let root = std::env::temp_dir().join(format!("rust-graph-bench-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for index in 0..files {
        let dir = root.join(format!("pkg{}", index % 10));
        fs::create_dir_all(&dir).unwrap();
        let (file, body) = match index % 3 {
            0 => (format!("mod{}_{index}.py", index % 100), python_file(index)),
            1 => (format!("f{}_{index}.js", index % 100), js_file(index)),
            _ => (format!("c{index}.c"), c_file(index)),
        };
        fs::write(dir.join(file), body).unwrap();
    }
    root
}

fn build(root: &Path, out: &Path) -> (Duration, String) {
    let start = Instant::now();
    let done = Command::new(BIN)
        .arg("build")
        .arg(root)
        .args(["--no-cache", "--out"])
        .arg(out)
        .output()
        .unwrap();
    let took = start.elapsed();
    assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
    (took, String::from_utf8_lossy(&done.stderr).into_owned())
}

fn is_static(binary: &[u8]) -> bool {
    // ELF64: a dynamically linked program has a PT_INTERP segment.
    const PT_INTERP: u32 = 3;
    let header_at = u64::from_le_bytes(binary[32..40].try_into().unwrap()) as usize;
    let entry_size = u16::from_le_bytes(binary[54..56].try_into().unwrap()) as usize;
    let entries = u16::from_le_bytes(binary[56..58].try_into().unwrap()) as usize;
    (0..entries).all(|index| {
        let at = header_at + index * entry_size;
        u32::from_le_bytes(binary[at..at + 4].try_into().unwrap()) != PT_INTERP
    })
}

fn peak_kb(stderr: &str) -> u64 {
    let word = stderr.rsplit("peak ").next().unwrap();
    word.split_whitespace().next().unwrap().parse().unwrap_or_else(|_| panic!("no peak in {stderr}"))
}

#[test]
fn the_binary_is_small_and_static() {
    let bytes = fs::read(BIN).unwrap();
    println!("binary: {} bytes, static: {}", bytes.len(), is_static(&bytes));
    assert!((bytes.len() as u64) < MAX_BINARY_BYTES, "{} bytes", bytes.len());
    if cfg!(target_env = "musl") {
        assert!(is_static(&bytes));
    }
}

#[test]
fn cold_start_is_under_the_limit() {
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        let start = Instant::now();
        assert!(Command::new(BIN).arg("--version").output().unwrap().status.success());
        best = best.min(start.elapsed());
    }
    println!("cold start: {best:?}");
    assert!(best < MAX_COLD_START, "{best:?}");
}

#[test]
fn a_thousand_files_build_inside_the_limit() {
    let root = tree("1k", 1000);
    let out = root.with_extension("out");
    let mut times: Vec<Duration> = (0..RUNS).map(|_| build(&root, &out).0).collect();
    times.sort_unstable();
    let bytes: u64 = fs::read_dir(root.join("pkg0")).unwrap().map(|e| e.unwrap().metadata().unwrap().len()).sum();
    println!("1,000 files (~{} KB in tree): median {:?}, best {:?}", bytes * 10 / 1024, times[RUNS / 2], times[0]);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&out);
    assert!(times[RUNS / 2] < MAX_THOUSAND_FILES, "{:?}", times[RUNS / 2]);
}

#[test]
fn ten_thousand_files_stay_inside_the_memory_limit() {
    let root = tree("10k", 10_000);
    let out = root.with_extension("out");
    let (took, stderr) = build(&root, &out);
    let peak = peak_kb(&stderr);
    println!("10,000 files: {took:?}, peak RSS {peak} KB, graph {} MB", fs::metadata(out.join("graph.json")).unwrap().len() / (1024 * 1024));
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&out);
    assert!(peak < MAX_PEAK_RSS_KB, "{peak} KB");
}
