//! A census of real Ruby: parse every `.rb` file under the paths given and
//! count the files a grammar change made worse or better.
//!
//! ```text
//! cargo run -p deep-grammars --example ruby_census -- DIR [DIR...]
//! ```
//!
//! The summary line is `clean broken [band0 band1 band2 band3 band4]`, where a
//! band buckets files by their share of error nodes (0, under 2%, under 10%,
//! under 40%, the rest), so a fix that turns two errors into one still moves a
//! file down a band. `--verbose` prints every broken file with its first error
//! and the source around it, which is how a construct is found from a corpus.

use std::fs;

use deep_grammars::batch_b::ruby::RUBY;
use deep_grammars::deep::syntax::ERROR;
use deep_grammars::deep::tree::{HAS_ERROR, TOKEN};
use deep_grammars::deep::{Budget, Workspace};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verbose = args.iter().any(|arg| arg == "--verbose");
    let mut files = Vec::new();
    for arg in args.iter().filter(|arg| arg.as_str() != "--verbose") {
        collect(arg, &mut files);
    }

    let mut workspace = Workspace::default();
    let (mut clean, mut broken) = (0, 0);
    let mut bands = [0usize; 5];
    for path in &files {
        let Ok(src) = fs::read(path) else { continue };
        if workspace.parse(&src, &RUBY, &Budget::default()).is_err() {
            continue;
        }
        let nodes = &workspace.tree.nodes;
        let bad = |index: usize| nodes[index].kind == ERROR || nodes[index].flags & HAS_ERROR != 0 && nodes[index].flags & TOKEN != 0;
        let errors = (0..nodes.len()).filter(|&index| bad(index)).count();
        let percent = errors * 100 / nodes.len().max(1);
        bands[match percent {
            0 => 0,
            1..=2 => 1,
            3..=10 => 2,
            11..=40 => 3,
            _ => 4,
        }] += 1;
        if errors == 0 {
            clean += 1;
            continue;
        }
        broken += 1;
        if verbose {
            let first = (1..nodes.len()).find(|&index| bad(index)).expect("a broken file has a first error");
            let node = &nodes[first];
            let line = 1 + src[..node.start as usize].iter().filter(|&&byte| byte == b'\n').count();
            let text = String::from_utf8_lossy(&src[node.start as usize..(node.end as usize).min(src.len())]);
            println!("{path}:{line}  {errors} errors, first: {}", text.chars().take(60).collect::<String>());
        }
    }
    if verbose {
        println!("clean={clean} broken={broken} bands={bands:?}");
    } else {
        println!("{clean} {broken} {bands:?}");
    }
}

fn collect(path: &str, out: &mut Vec<String>) {
    let Ok(meta) = fs::metadata(path) else { return };
    if meta.is_dir() {
        let Ok(entries) = fs::read_dir(path) else { return };
        let mut names: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        names.sort();
        for name in names {
            collect(&name.to_string_lossy(), out);
        }
    } else if path.ends_with(".rb") {
        out.push(path.to_string());
    }
}
