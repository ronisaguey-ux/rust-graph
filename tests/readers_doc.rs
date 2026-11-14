//! `docs/READERS.md` against the coverage tables, both directions.
//!
//! The document lists every construct each reader claims. A hand-written list
//! drifts the first time a construct is added or a gap is closed, and a list
//! that overstates what the readers do is worse than no list at all, because it
//! is what a person reads instead of the tests. This test reads the tables out
//! of the test sources and the bullets out of the document, and fails if either
//! side has something the other does not.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const TABLES: [&str; 3] = [
    "tests/js_coverage.rs",
    "tests/ts_coverage.rs",
    "tests/python_coverage.rs",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// The first string literal of every tuple in `const NAME: …`, which is the
/// construct's name in both tables.
fn constructs(source: &str, name: &str) -> Vec<String> {
    let marker = format!("const {name}:");
    let Some(start) = source.find(&marker) else {
        return Vec::new();
    };
    let rest = &source[start..];
    // Bracket depth, not a search for `];`: an empty table closes as `&[];` and
    // a search for the first line that starts with `];` would run past it into
    // whatever code comes next.
    let body = rest.as_bytes();
    // The array literal starts after `= &`; the first bracket on the line is the
    // slice type, `&[(&str, &str)]`, and its closing bracket would end the scan
    // before a single tuple was read.
    let open = match rest.find("= &").and_then(|index| rest[index..].find('[')) {
        Some(index) => index + rest.find("= &").unwrap_or(0),
        None => return Vec::new(),
    };
    let mut depth = 0i32;
    let mut end = body.len();
    let mut index = open;
    let mut in_string = false;
    while index < body.len() {
        match body[index] {
            b'"' => in_string = !in_string,
            b'\\' if in_string => index += 1,
            b'[' if !in_string => depth += 1,
            b']' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    end = index;
                    break;
                }
            }
            _ => {}
        }
        index += 1;
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < end {
        if body[i] == b'(' {
            let mut j = i + 1;
            while j < end && body[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < end && body[j] == b'"' {
                j += 1;
                let mut text = String::new();
                let mut escaped = false;
                while j < end && (escaped || body[j] != b'"') {
                    if escaped {
                        text.push(match body[j] {
                            b'n' => '\n',
                            b't' => '\t',
                            other => other as char,
                        });
                        escaped = false;
                    } else if body[j] == b'\\' {
                        escaped = true;
                    } else {
                        text.push(body[j] as char);
                    }
                    j += 1;
                }
                out.push(text);
            }
        }
        i += 1;
    }
    out
}

fn claimed() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for table in TABLES {
        let source = std::fs::read_to_string(root().join(table)).expect("a coverage table");
        out.extend(constructs(&source, "COVERAGE"));
        out.extend(constructs(&source, "KNOWN_GAPS"));
    }
    out
}

fn documented(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|line| line.strip_prefix("- "))
        // The prose sections name a file and a test, not a construct.
        .filter(|line| !line.starts_with("**") && !line.starts_with("none:") && !line.starts_with("nothing "))
        .map(|line| line.trim().to_string())
        .collect()
}

#[test]
fn the_document_lists_exactly_what_the_tests_claim() {
    let text = std::fs::read_to_string(root().join("docs/READERS.md")).expect("docs/READERS.md");
    let claimed = claimed();
    let documented = documented(&text);
    assert!(!claimed.is_empty(), "no construct was read out of the coverage tables");

    let missing: Vec<&String> = claimed.difference(&documented).collect();
    assert!(
        missing.is_empty(),
        "claimed by a test but not listed in docs/READERS.md: {missing:?}"
    );
    let unclaimed: Vec<&String> = documented.difference(&claimed).collect();
    assert!(
        unclaimed.is_empty(),
        "listed in docs/READERS.md but no test claims it: {unclaimed:?}"
    );
}
