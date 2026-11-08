//! `compare` and `memory`: the two measurements. Both time a whole process, so
//! neither needs to know how a parser works, and both refuse rather than report
//! when the thing they were asked to measure is not there.

use std::process::Command;
use std::time::{Duration, Instant};

use super::manifest::Manifest;
use super::{check_program, describe, environment, tail, Options, EXIT_GATE, MIN_SPEEDUP};

// ---------------------------------------------------------------- comparison

/// Run both parsers `--rounds` times, alternating which goes first, and compare
/// their medians. The gate is a property of the medians, not of any one round.
pub(super) fn compare(options: &Options, argv: &[String]) -> Result<i32, String> {
    let tree_sitter = options.program("--tree-sitter", &options.tree_sitter)?;
    let deep = options.program("--deep", &options.deep)?;
    let manifest = Manifest::load(options.list()?)?;
    let rounds = options.rounds.unwrap_or(5);
    check_program(tree_sitter, "--tree-sitter")?;
    check_program(deep, "--deep")?;

    environment(argv);
    println!("{}", manifest.summary());
    println!("rounds: {rounds}  gate: {MIN_SPEEDUP:.1}x  timing: whole process, wall clock");
    println!("tree-sitter: {tree_sitter}");
    println!("deep:        {deep}");
    let reps = match options.reps {
        Some(reps) => format!(" --reps {reps}"),
        None => String::new(),
    };
    println!("parser contract: <binary> --list {}{reps}", manifest.path);
    println!();

    let mut tree_sitter_ms = Vec::with_capacity(rounds);
    let mut deep_ms = Vec::with_capacity(rounds);
    for round in 0..rounds {
        // Alternating the order is what keeps a warming page cache or a
        // frequency drop from favouring whichever parser always went first.
        // Both numbers are labelled afterwards, so the log reads in a fixed
        // order whichever parser went first.
        let tree_sitter_first = round % 2 == 0;
        let first = if tree_sitter_first { tree_sitter } else { deep };
        let first_ms = run_once(first, options, &manifest.path).map_err(|error| format!("round {}: {error}", round + 1))?;
        let second_ms = run_once(if tree_sitter_first { deep } else { tree_sitter }, options, &manifest.path)
            .map_err(|error| format!("round {}: {error}", round + 1))?;
        let (tree, deep_ms_run) = if tree_sitter_first { (first_ms, second_ms) } else { (second_ms, first_ms) };
        tree_sitter_ms.push(tree);
        deep_ms.push(deep_ms_run);
        println!(
            "round {:>2}  {}  tree-sitter {:>9.3} ms  deep {:>9.3} ms",
            round + 1,
            if tree_sitter_first { "ts,deep" } else { "deep,ts" },
            tree,
            deep_ms_run
        );
    }
    println!();

    let (ts, dp) = (stats(tree_sitter_ms), stats(deep_ms));
    println!("{:<12} {:>10} {:>10} {:>10} {:>10}  (ms)", "parser", "median", "mean", "min", "max");
    println!("{:<12} {:>10.3} {:>10.3} {:>10.3} {:>10.3}", "tree-sitter", ts.median, ts.mean, ts.min, ts.max);
    println!("{:<12} {:>10.3} {:>10.3} {:>10.3} {:>10.3}", "deep", dp.median, dp.mean, dp.min, dp.max);
    if dp.median <= 0.0 {
        return Err("the deep parser measured 0 ms, so no ratio exists".into());
    }
    let ratio = ts.median / dp.median;
    let throughput = |ms: f64| manifest.bytes as f64 / 1e6 / (ms / 1e3);
    println!();
    println!("deep throughput: {:.1} MB/s  tree-sitter: {:.1} MB/s", throughput(dp.median), throughput(ts.median));
    if ratio < MIN_SPEEDUP {
        println!("ratio (tree-sitter / deep): {ratio:.2}x  gate {MIN_SPEEDUP:.1}x  FAIL");
        Ok(EXIT_GATE)
    } else {
        println!("ratio (tree-sitter / deep): {ratio:.2}x  gate {MIN_SPEEDUP:.1}x  PASS");
        Ok(0)
    }
}

/// One measured run: spawn, wait, and hand back the wall time in ms. Process
/// start-up is inside the number on both sides, which is why both sides pay it.
fn run_once(program: &str, options: &Options, list: &str) -> Result<f64, String> {
    let mut command = Command::new(program);
    command.arg("--list").arg(list);
    if let Some(reps) = options.reps {
        command.arg("--reps").arg(reps.to_string());
    }
    let shown = describe(&command);
    let start = Instant::now();
    let output = command.output().map_err(|error| format!("cannot run {shown}: {error}"))?;
    let elapsed = start.elapsed().as_secs_f64() * 1e3;
    if !output.status.success() {
        return Err(format!("{shown} failed ({}):\n{}", output.status, tail(&output.stderr)));
    }
    Ok(elapsed)
}

#[derive(Debug, PartialEq)]
pub(super) struct Stats {
    pub(super) median: f64,
    pub(super) mean: f64,
    pub(super) min: f64,
    pub(super) max: f64,
}

pub(super) fn stats(mut values: Vec<f64>) -> Stats {
    assert!(!values.is_empty(), "stats needs at least one measurement");
    values.sort_by(f64::total_cmp);
    let count = values.len();
    let median = if count % 2 == 1 { values[count / 2] } else { (values[count / 2 - 1] + values[count / 2]) / 2.0 };
    Stats { median, mean: values.iter().sum::<f64>() / count as f64, min: values[0], max: values[count - 1] }
}

// ------------------------------------------------------------------- memory

pub(super) fn memory(options: &Options, argv: &[String]) -> Result<i32, String> {
    let deep = options.program("--deep", &options.deep)?;
    let manifest = Manifest::load(options.list()?)?;
    check_program(deep, "--deep")?;

    environment(argv);
    println!("{}", manifest.summary());
    println!("deep:        {deep}");
    println!("peak rss source: /proc/<pid>/status VmHWM, sampled every 1 ms while the parser lives");
    println!();

    let mut command = Command::new(deep);
    command.arg("--list").arg(manifest.path.as_str());
    let shown = describe(&command);
    let start = Instant::now();
    let mut child = command.spawn().map_err(|error| format!("cannot run {shown}: {error}"))?;
    let pid = child.id();
    let (mut peak, mut samples) = (0u64, 0u64);
    let status = loop {
        if let Some(kb) = vm_hwm(pid) {
            peak = peak.max(kb);
            samples += 1;
        }
        match child.try_wait().map_err(|error| format!("waiting for {shown}: {error}"))? {
            Some(status) => break status,
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    };
    let elapsed = start.elapsed().as_secs_f64() * 1e3;
    if !status.success() {
        return Err(format!("{shown} failed ({status})"));
    }
    if samples == 0 {
        return Err(format!(
            "{shown} finished in {elapsed:.1} ms without a readable VmHWM sample, so its peak RSS is unknown; give it a bigger corpus and try again"
        ));
    }
    println!("{shown}");
    println!("parsed:        {} files, {} bytes in {elapsed:.1} ms", manifest.files.len(), manifest.bytes);
    println!("peak rss:      {peak} kB = {:.1} MB  ({} samples of /proc/{pid}/status VmHWM)", peak as f64 / 1024.0, samples);
    match vm_hwm(std::process::id()) {
        Some(own) => println!("bench own peak rss (VmHWM): {:.1} MB", own as f64 / 1024.0),
        None => println!("bench own peak rss (VmHWM): unreadable"),
    }
    Ok(0)
}

/// `VmHWM` — the high-water mark of resident memory — in kB. `VmHWM` is the same
/// field `/proc/self/status` shows, read through `/proc/<pid>/status` so the
/// number belongs to the parser and not to this harness: a process that has
/// already exited has no peak left to read.
pub(super) fn vm_hwm(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|kb| kb.parse().ok())
}
