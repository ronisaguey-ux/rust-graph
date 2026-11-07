//! rust-graph: a code knowledge graph. See README.md.

mod alloc;
mod expr;
mod facts;
mod index;
mod json;
mod langs;
mod lexer;
mod parser;
mod pipeline;
mod resolve;
mod sha256;

use std::path::PathBuf;
use std::process::ExitCode;

use pipeline::Options;

#[global_allocator]
static POOL: alloc::Pool = alloc::Pool::new();

const USAGE: &str = "usage: rust-graph build <root> [--out DIR] [--stdout] [--cache DIR] [--no-cache]
                        [--max-files N] [--max-file-bytes N]
       rust-graph languages
       rust-graph --version";

fn number(value: Option<String>, flag: &str) -> Result<u64, String> {
    value.and_then(|text| text.parse().ok()).ok_or_else(|| format!("{flag} needs a number"))
}

fn options(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut args = args;
    let mut root = None;
    let mut options = Options::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => options.out = Some(PathBuf::from(args.next().ok_or("--out needs a directory")?)),
            "--stdout" => options.stdout = true,
            "--cache" => options.cache = Some(PathBuf::from(args.next().ok_or("--cache needs a directory")?)),
            "--no-cache" => options.no_cache = true,
            "--max-files" => options.max_files = number(args.next(), "--max-files")? as usize,
            "--max-file-bytes" => options.max_file_bytes = number(args.next(), "--max-file-bytes")?,
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            path if root.is_none() => root = Some(PathBuf::from(path)),
            extra => return Err(format!("unexpected argument {extra}")),
        }
    }
    options.root = root.ok_or("build needs a root directory")?;
    Ok(options)
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("build") => options(args).and_then(|options| pipeline::build(&options).map_err(|error| error.to_string())),
        Some("languages") => {
            for (suffix, tag, name) in langs::all_suffixes() {
                println!(".{suffix}\t{tag}\t{name}");
            }
            Ok(())
        }
        Some("--version") => {
            println!("rust-graph {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("rust-graph: {message}");
            ExitCode::from(2)
        }
    }
}
