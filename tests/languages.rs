//! Every language in the table, through the real binary: a caller, a callee, an
//! edge between them. A row that stops parsing fails here by name.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn scratch(name: &str) -> PathBuf {
    let run = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rust-graph-lang-{}-{run}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn graph(file: &str, body: &str) -> String {
    let dir = scratch(&file.replace('.', "_"));
    std::fs::write(dir.join(file), body).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_rust-graph"))
        .args(["build", dir.to_str().unwrap(), "--stdout", "--no-cache"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8(out.stdout).unwrap()
}

const SAMPLES: &[(&str, &str, &str)] = &[
    ("Python", "a.py", "def callee(x):\n    return x\n\ndef caller(y):\n    return callee(y)\n"),
    ("JavaScript", "a.js", "function callee(x) {\n  return x;\n}\nfunction caller(y) {\n  return callee(y);\n}\n"),
    ("TypeScript", "a.ts", "function callee(x: string): string {\n  return x;\n}\nexport const caller = (y: string) => callee(y);\n"),
    ("C", "a.c", "int callee(int x) {\n  return x;\n}\nint caller(int y) {\n  return callee(y);\n}\n"),
    ("C++", "a.cpp", "int callee(int x) {\n  return x;\n}\nint caller(int y) {\n  return callee(y);\n}\n"),
    ("Objective-C", "a.mm", "int callee(int x) {\n  return x;\n}\n- (int)caller:(int)y {\n  return callee(y);\n}\n"),
    ("MATLAB", "a.m", "function y = callee(x)\n  y = x;\nend\nfunction z = caller(w)\n  z = callee(w);\nend\n"),
    ("Perl", "a.pm", "sub callee {\n  my ($x) = @_;\n  return $x;\n}\nsub caller {\n  return callee($_[0]);\n}\n"),
    ("Go", "a.go", "package a\n\nfunc callee(x string) string {\n\treturn x\n}\n\nfunc caller(y string) string {\n\treturn callee(y)\n}\n"),
    ("Rust", "a.rs", "fn callee(x: u32) -> u32 {\n    x\n}\nfn caller(y: u32) -> u32 {\n    callee(y)\n}\n"),
    ("PHP", "a.php", "<?php\nfunction callee($x) {\n  return $x;\n}\nfunction caller($y) {\n  return callee($y);\n}\n"),
    ("Java", "A.java", "class A {\n  static int callee(int x) {\n    return x;\n  }\n  int caller(int y) {\n    return callee(y);\n  }\n}\n"),
    ("Kotlin", "a.kt", "fun callee(x: Int): Int {\n    return x\n}\nfun caller(y: Int) = callee(y)\n"),
    ("C#", "a.cs", "class A {\n  int callee(int x) {\n    return x;\n  }\n  int caller(int y) {\n    return callee(y);\n  }\n}\n"),
    ("Ruby", "a.rb", "def callee(x)\n  x\nend\n\ndef caller(y)\n  callee(y)\nend\n"),
    ("Swift", "a.swift", "func callee(x: Int) -> Int {\n    return x\n}\nfunc caller(y: Int) -> Int {\n    return callee(x: y)\n}\n"),
    ("Scala", "a.scala", "object A {\n  def callee(x: Int): Int = {\n    x\n  }\n  def caller(y: Int): Int = this.callee(y)\n}\n"),
    ("Dart", "a.dart", "int callee(int x) {\n  return x;\n}\nint caller(int y) {\n  return callee(y);\n}\n"),
    ("Elixir", "a.ex", "defmodule A do\n  def callee(x) do\n    x\n  end\n\n  def caller(y) do\n    callee(y)\n  end\nend\n"),
    ("Haskell", "a.hs", "module A where\n\ncallee :: Int -> Int\ncallee x = x\n\ncaller y = callee y\n"),
    ("Lua", "a.lua", "function callee(x)\n  return x\nend\n\nlocal function caller(y)\n  return callee(y)\nend\n"),
    ("R", "a.r", "callee <- function(x) {\n  x\n}\ncaller <- function(y) {\n  callee(y)\n}\n"),
    ("Solidity", "a.sol", "contract A {\n  function callee(uint x) public returns (uint) {\n    return x;\n  }\n  function caller(uint y) public returns (uint) {\n    return callee(y);\n  }\n}\n"),
    ("SQL", "a.sql", "CREATE FUNCTION callee(x INT) RETURNS INT\nRETURN x;\nCREATE PROCEDURE caller(y INT)\n  SELECT callee(y);\n"),
    ("Shell", "a.sh", "callee() {\n  echo \"$1\"\n}\ncaller() {\n  callee \"$1\"\n}\n"),
    ("PowerShell", "a.ps1", "function Callee($x) {\n  return $x\n}\nfunction Caller($y) {\n  Callee $y\n}\n"),
    ("Groovy", "a.groovy", "def callee(x) {\n  return x\n}\ndef caller(y) {\n  return callee(y)\n}\n"),
    ("Julia", "a.jl", "function callee(x)\n  x\nend\nfunction caller(y)\n  callee(y)\nend\n"),
    ("Visual Basic", "a.vb", "Function Callee(x)\n  Callee = x\nEnd Function\nSub Caller(y)\n  Call Callee(y)\nEnd Sub\n"),
    ("Pascal", "a.pas", "function Callee(x: Integer): Integer;\nbegin\n  Result := x;\nend;\n\nprocedure Caller(y: Integer);\nbegin\n  Callee(y);\nend;\n"),
    ("Fortran", "a.f90", "subroutine callee(x)\n  print *, x\nend subroutine callee\nsubroutine caller(y)\n  call callee(y)\nend subroutine caller\n"),
    ("COBOL", "a.cob", "       PROCEDURE DIVISION.\n       CALLEE.\n           DISPLAY X.\n       CALLER.\n           PERFORM CALLEE.\n"),
    ("Nim", "a.nim", "proc callee(x: int): int =\n  x\n\nproc caller(y: int): int =\n  callee(y)\n"),
    ("Crystal", "a.cr", "def callee(x)\n  x\nend\n\ndef caller(y)\n  callee(y)\nend\n"),
    ("Zig", "a.zig", "fn callee(x: u32) u32 {\n    return x;\n}\nfn caller(y: u32) u32 {\n    return callee(y);\n}\n"),
    ("D", "a.d", "int callee(int x) {\n  return x;\n}\nint caller(int y) {\n  return callee(y);\n}\n"),
    ("F#", "a.fs", "let callee x = x\n\nlet caller y = callee y\n"),
    ("OCaml", "a.ml", "let callee x = x\n\nlet caller y = callee y\n"),
    ("Erlang", "a.erl", "callee(X) -> X.\n\ncaller(Y) -> callee(Y).\n"),
    ("Clojure", "a.clj", "(defn callee [x] x)\n\n(defn caller [y] (callee y))\n"),
    ("Elm", "a.elm", "module A exposing (..)\n\ncallee x = x\n\ncaller y = callee y\n"),
    ("Tcl", "a.tcl", "proc callee {x} {\n  return $x\n}\nproc caller {y} {\n  callee $y\n}\n"),
    ("AWK", "a.awk", "function callee(x) {\n  return x\n}\nfunction caller(y) {\n  return callee(y)\n}\n"),
];

const NOISE: &[u8] = b"(){}[]'\"`\\/\n =:;,.#$@f x";

fn has_edge(json: &str) -> bool {
    json.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.starts_with("{\"caller\":") && lower.contains("caller\",\"callee\":") && lower.contains("callee\",\"line\"")
    })
}

#[test]
fn every_language_yields_a_caller_a_callee_and_the_edge() {
    let mut failed = Vec::new();
    for (name, file, body) in SAMPLES {
        if !has_edge(&graph(file, body)) {
            failed.push(*name);
        }
    }
    assert!(failed.is_empty(), "no caller→callee edge for: {failed:?}");
    assert!(SAMPLES.len() >= 40, "{}", SAMPLES.len());
}

#[test]
fn a_file_with_no_grammar_here_adds_no_symbols() {
    assert!(graph("facts.pl", "parent(tom, bob).\nanc(X, Y) :- parent(X, Y).\n").contains("\"files\":0"));
}

#[test]
fn hostile_input_does_not_crash_the_parser() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for (_, file, sample) in SAMPLES {
        for round in 0..4 {
            let mut body = String::new();
            for _ in 0..4000 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                body.push(NOISE[(seed % NOISE.len() as u64) as usize] as char);
                if round == 3 && seed % 9 == 0 {
                    body.push_str(sample);
                }
            }
            graph(file, &body);
        }
        for cut in 0..sample.len() {
            if sample.is_char_boundary(cut) && cut % 7 == 0 {
                graph(file, &sample[..cut]);
            }
        }
    }
}
