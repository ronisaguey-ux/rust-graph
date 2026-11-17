//! Dump the CST of one Kotlin file as an s-expression:
//! `cargo run -p deep-grammars --example kt_dump -- FILE`.
use std::fs;

use deep_grammars::batch_a::kotlin::KOTLIN;
use deep_grammars::deep::{Budget, Workspace};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let src = fs::read(&args[0]).unwrap();
    let named_only = args.iter().any(|arg| arg == "--named");
    let mut workspace = Workspace::default();
    if workspace.parse(&src, &KOTLIN, &Budget::default()).is_err() {
        println!("parse failed");
        return;
    }
    println!("{}", workspace.tree.sexp(&KOTLIN, named_only));
}
