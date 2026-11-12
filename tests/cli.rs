//! Behaviour of the whole build: determinism, the cache, the output contract.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_rust-graph");

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str, files: &[(&str, &str)]) -> Tree {
        let dir = std::env::temp_dir().join(format!("rust-graph-cli-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (path, body) in files {
            let target = dir.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, body).unwrap();
        }
        Tree(dir)
    }

    fn build(&self, extra: &[&str]) -> Output {
        let out = Command::new(BIN).arg("build").arg(&self.0).args(extra).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        out
    }

    fn json(&self) -> String {
        String::from_utf8(self.build(&["--stdout", "--no-cache"]).stdout).unwrap()
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const TWO_FILES: &[(&str, &str)] = &[
    ("app/a.py", "from app.b import helper\n\n\ndef f(x):\n    return helper(x)\n"),
    ("app/b.py", "def helper(y):\n    return y\n"),
];

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn two_builds_and_two_locations_give_the_same_bytes() {
    let one = Tree::new("det-one", TWO_FILES);
    let two = Tree::new("det-two", TWO_FILES);
    assert_eq!(one.json(), one.json());
    assert_eq!(one.json(), two.json());
}

#[test]
fn edges_say_how_they_were_resolved() {
    let tree = Tree::new("tags", TWO_FILES);
    let json = tree.json();
    assert!(json.contains(r#""caller":"app/a.py:f","callee":"app/b.py:helper","line":5,"kind":"call","tag":"EXTRACTED","strategy":"import""#), "{json}");
    assert!(json.contains(r#""module":"app.b","name":"helper","alias":"helper","star":false,"target":"app/b.py","tag":"EXTRACTED""#), "{json}");
}

#[test]
fn a_name_implied_by_the_tree_is_inferred() {
    let tree = Tree::new("inferred", &[
        ("src/a.c", "int main(void) {\n  return helper(1);\n}\n"),
        ("src/b.c", "int helper(int x) {\n  return x;\n}\n"),
    ]);
    assert!(tree.json().contains(r#""tag":"INFERRED","strategy":"same-directory""#));
}

#[test]
fn an_unchanged_tree_parses_nothing_and_gives_the_same_graph() {
    let tree = Tree::new("cache", TWO_FILES);
    let cache = tree.0.join(".rust-graph");
    let cold = tree.build(&["--stdout"]);
    assert!(stderr(&cold).contains("2 parsed, 0 cached"), "{}", stderr(&cold));
    let warm = tree.build(&["--stdout"]);
    assert!(stderr(&warm).contains("0 parsed, 2 cached"), "{}", stderr(&warm));
    assert_eq!(cold.stdout, warm.stdout);
    assert!(cache.join("facts.pack").exists());
}

#[test]
fn an_edit_reparses_that_file_and_re_resolves_its_callers() {
    let tree = Tree::new("edit", TWO_FILES);
    tree.build(&["--stdout"]);
    fs::write(tree.0.join("app/b.py"), "def other(y):\n    return y\n").unwrap();
    let out = tree.build(&["--stdout"]);
    assert!(stderr(&out).contains("1 parsed, 1 cached"), "{}", stderr(&out));
    let json = String::from_utf8(out.stdout).unwrap();
    assert!(!json.contains(r#""callee":"app/b.py:helper""#), "a stale edge survived: {json}");
    assert_eq!(json, tree.json(), "a cached build must equal a clean one");
}

#[test]
fn a_corrupt_cache_is_an_empty_cache() {
    let tree = Tree::new("corrupt", TWO_FILES);
    fs::create_dir_all(tree.0.join(".rust-graph")).unwrap();
    fs::write(tree.0.join(".rust-graph/facts.pack"), b"RGPACK01\x01\0\0\0garbage").unwrap();
    let out = tree.build(&["--stdout"]);
    assert!(stderr(&out).contains("2 parsed, 0 cached"), "{}", stderr(&out));
}

#[test]
fn the_default_output_is_a_graph_and_a_report() {
    let tree = Tree::new("report", TWO_FILES);
    let out = tree.0.join("out");
    tree.build(&["--out", out.to_str().unwrap(), "--no-cache"]);
    assert!(fs::read_to_string(out.join("graph.json")).unwrap().starts_with("{\"version\":1"));
    let report = fs::read_to_string(out.join("report.md")).unwrap();
    assert!(report.contains("2 files, 2 symbols, 1 call edges (1 EXTRACTED, 0 INFERRED)"), "{report}");
    assert!(report.contains("app/b.py:helper"), "{report}");
}

#[test]
fn the_file_limit_is_stated_in_the_output() {
    let tree = Tree::new("limit", TWO_FILES);
    let json = String::from_utf8(tree.build(&["--stdout", "--no-cache", "--max-files", "1"]).stdout).unwrap();
    assert!(json.contains("\"files\":1") && json.contains("\"truncated\":true"), "{json}");
}

#[test]
fn an_oversized_file_is_counted_not_read() {
    let tree = Tree::new("large", &[("big.py", "def f():\n    return 1\n")]);
    let json = String::from_utf8(tree.build(&["--stdout", "--no-cache", "--max-file-bytes", "4"]).stdout).unwrap();
    assert!(json.contains("\"too_large\":1") && !json.contains("big.py:f"), "{json}");
}

#[test]
fn a_missing_root_is_an_error() {
    let out = Command::new(BIN).args(["build", "/nonexistent/tree", "--stdout"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("not a directory"));
}

#[test]
fn a_symlink_is_not_followed_out_of_the_tree() {
    let outside = Tree::new("outside", &[("secret.py", "def leak():\n    return 1\n")]);
    let tree = Tree::new("links", TWO_FILES);
    std::os::unix::fs::symlink(Path::new(&outside.0), tree.0.join("link")).unwrap();
    assert!(!tree.json().contains("leak"));
}

#[test]
fn a_named_pipe_or_device_link_does_not_block_the_build() {
    let tree = Tree::new("pipes", TWO_FILES);
    let made = Command::new("mkfifo").arg(tree.0.join("pipe.py")).status().unwrap();
    assert!(made.success());
    std::os::unix::fs::symlink("/dev/zero", tree.0.join("zero.py")).unwrap();
    let mut child = Command::new(BIN).arg("build").arg(&tree.0).args(["--no-cache", "--stdout"]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
    for _ in 0..500 {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    child.kill().unwrap();
    panic!("the build blocked on a FIFO");
}

#[test]
fn flows_record_how_values_move() {
    let tree = Tree::new("flows", &[("a.py", "def f(x):\n    a, b = g(x), 1\n    return f\"{a}\"\n")]);
    let json = tree.json();
    assert!(json.contains(r#"["a",2,["a","b"],["t",[["c",0,0,[["n","x"]]],0]],0]"#), "{json}");
    assert!(json.contains(r#"["r",3,["n","a"]]"#), "{json}");
}

#[test]
fn a_bare_name_in_a_class_is_a_member_only_where_the_language_says_so() {
    let java = Tree::new("member-java", &[("A.java", "class A {\n  int f() {\n    return g();\n  }\n  int g() {\n    return 1;\n  }\n}\n")]);
    assert!(java.json().contains(r#""caller":"A.java:A.f","callee":"A.java:A.g","line":3,"kind":"call","tag":"EXTRACTED","strategy":"same-class""#));
    let python = Tree::new("member-py", &[("a.py", "class A:\n    def f(self):\n        return g()\n\n    def g(self):\n        return 1\n")]);
    assert!(!python.json().contains(r#""callee":"a.py:A.g""#));
}

#[test]
fn an_assigned_function_is_named_for_the_word_beside_the_equals() {
    let tree = Tree::new("assigned", &[("a.ts", "export const one = (x: string) => x;\nconst two: Fn = (y: string) => y;\nexports.three = (z) => z;\n")]);
    let json = tree.json();
    // `exports.three` keeps the target's path: naming it `three` collided with
    // any other `x.three` binding in the same file and merged their flows.
    for name in ["a.ts:one", "a.ts:two", "a.ts:exports.three"] {
        assert!(json.contains(&format!("\"id\":\"{name}\"")), "{name}: {json}");
    }
}

#[test]
fn a_sql_body_in_column_one_stays_in_its_routine() {
    let tree = Tree::new("sql", &[("a.sql", "CREATE OR REPLACE FUNCTION app.f(x INT) RETURNS INT AS $$\nBEGIN\nPERFORM g(x);\nEND;\n$$ LANGUAGE plpgsql;\nCREATE TABLE t (a INT);\nCREATE FUNCTION g(y INT) RETURNS INT\nRETURN y;\n")]);
    let json = tree.json();
    assert!(json.contains(r#""caller":"a.sql:app.f","callee":"a.sql:g""#), "{json}");
}

#[test]
fn an_erlang_clause_runs_to_its_period_across_lines() {
    let tree = Tree::new("erlang", &[("a.erl", "-module(a).\nf(X) ->\n    Y = g(X),\n    h(Y).\ng(A) -> A.\nh(B) -> B.\n")]);
    let json = tree.json();
    assert!(json.contains(r#""caller":"a.erl:f","callee":"a.erl:g""#), "{json}");
    assert!(json.contains(r#""caller":"a.erl:f","callee":"a.erl:h""#), "{json}");
}

/// Build `tree`, killing it past `limit`: a regression must fail, not stall.
fn build_within(tree: &Tree, limit: Duration) -> String {
    let dir = std::env::temp_dir().join(format!("rust-graph-cli-{}-out", tree.0.file_name().unwrap().to_string_lossy()));
    let out = dir.with_extension("json");
    let mut child = Command::new(BIN)
        .arg("build")
        .arg(&tree.0)
        .args(["--stdout", "--no-cache"])
        .stdout(fs::File::create(&out).unwrap())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > limit {
            child.kill().unwrap();
            child.wait().unwrap();
            let _ = fs::remove_file(&out);
            panic!("build still running after {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let json = fs::read_to_string(&out).unwrap();
    fs::remove_file(&out).unwrap();
    json
}

/// `count` files or classes, each pointing at all the others.
fn others(count: usize, line: impl Fn(usize) -> String) -> impl Fn(usize) -> String {
    move |me| (0..count).filter(|other| *other != me).map(&line).collect()
}

const CYCLE_LIMIT: Duration = Duration::from_secs(5);

#[test]
fn cyclic_bases_do_not_stall_the_build() {
    let count = 10;
    let bases = others(count, |other| format!("C{other}, "));
    let source: String = (0..count).map(|me| format!("class C{me}({}):\n    def go(self):\n        self.missing()\n\n", bases(me))).collect();
    let tree = Tree::new("cycle-bases", &[("a.py", &source)]);
    let json = build_within(&tree, CYCLE_LIMIT);
    assert!(json.contains("a.py:C9.go"), "{json}");
}

#[test]
fn cyclic_aliased_imports_do_not_stall_the_build() {
    let count = 10;
    let imports = others(count, |other| format!("from m{other} import x\n"));
    let files: Vec<(String, String)> =
        (0..count).map(|me| (format!("m{me}.py"), format!("{}def f{me}():\n    x()\n", imports(me)))).collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(path, body)| (path.as_str(), body.as_str())).collect();
    let tree = Tree::new("cycle-alias", &refs);
    let json = build_within(&tree, CYCLE_LIMIT);
    assert!(json.contains("m9.py:f9"), "{json}");
}

#[test]
fn cyclic_star_exports_do_not_stall_the_build() {
    let count = 10;
    let exports = others(count, |other| format!("export * from './m{other}';\n"));
    let files: Vec<(String, String)> =
        (0..count).map(|me| (format!("m{me}.js"), format!("{}function f{me}() {{\n  nothing_here();\n}}\n", exports(me)))).collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(path, body)| (path.as_str(), body.as_str())).collect();
    let tree = Tree::new("cycle-star", &refs);
    let json = build_within(&tree, CYCLE_LIMIT);
    assert!(json.contains("m9.js:f9"), "{json}");
}

#[test]
fn a_method_inside_a_base_cycle_still_binds() {
    let source = "class A(B):\n    def go(self):\n        self.run()\n\nclass B(A, C):\n    pass\n\nclass C(A):\n    def run(self):\n        pass\n";
    let tree = Tree::new("cycle-found", &[("a.py", source)]);
    let json = build_within(&tree, CYCLE_LIMIT);
    assert!(json.contains(r#""callee":"a.py:C.run""#), "{json}");
}

/// `X` first fails one step above the depth cap, where the walk stops short of
/// `Z`. That failure may only rule out deeper visits, so the short path still binds.
#[test]
fn a_class_first_met_near_the_depth_cap_stays_reachable() {
    let mut source = String::from("class A(L1, X):\n    def go(self):\n        self.run()\n\n");
    for step in 1..=5 {
        source.push_str(&format!("class L{step}(L{}):\n    pass\n\n", step + 1));
    }
    source.push_str("class L6(X):\n    pass\n\nclass X(Y):\n    pass\n\nclass Y(Z):\n    pass\n\nclass Z:\n    def run(self):\n        pass\n");
    let tree = Tree::new("cycle-cap", &[("a.py", &source)]);
    let json = build_within(&tree, CYCLE_LIMIT);
    assert!(json.contains(r#""callee":"a.py:Z.run""#), "{json}");
}

#[test]
fn a_python_star_import_ends_at_its_line() {
    let tree = Tree::new("py-star", &[
        ("a.py", "from m import *\ndef f():\n    helper()\n"),
        ("m.py", "def helper():\n    pass\n"),
    ]);
    let json = tree.json();
    assert!(json.contains(r#""id":"a.py:f""#), "{json}");
    assert!(json.contains(r#""caller":"a.py:f","callee":"m.py:helper","line":3,"kind":"call","tag":"EXTRACTED","strategy":"star-import""#), "{json}");
    assert_eq!(json.matches(r#""file":"a.py","line":1,"module":"m""#).count(), 1, "one import, not a bogus `:` one: {json}");
}
