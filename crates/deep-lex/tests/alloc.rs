//! The allocation claim, measured instead of asserted.
//!
//! A counting `GlobalAlloc` wraps the system allocator, and the whole token
//! loop runs with the counter live. This file deliberately contains a single
//! test: a test binary runs tests on parallel threads, and a counter shared
//! with another test would not mean anything.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use deep_lex::{DeepLexer, Language, Lexer};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        System.realloc(ptr, layout, new_size)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        System.alloc_zeroed(layout)
    }
}

#[global_allocator]
static A: Counting = Counting;

fn count() -> usize {
    ALLOCS.load(Ordering::SeqCst)
}

#[test]
fn one_allocation_for_the_whole_lexer_and_none_per_token() {
    // Built before any measurement: the input itself and the fixture text.
    let mut js = Vec::new();
    for _ in 0..2_000 {
        js.extend_from_slice(b"let x = \"v\"; if (a) { b = c / 2; }\n");
    }
    let py = std::fs::read(format!(
        "{}/tests/fixtures/nested.py",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let no_brackets = b"plain identifiers and numbers 42 here\n";

    // 1. A bracket-free source: the pair Vec stays empty, so construction is
    //    allocation-free too.
    let before = count();
    let mut lexer = DeepLexer::new(no_brackets, Language::JavaScript);
    let mut n = 0usize;
    while lexer.next_token(0).is_some() {
        n += 1;
    }
    let after = count();
    assert!(n > 0);
    assert_eq!(
        after - before,
        0,
        "no brackets and no tokens = not a single allocation"
    );

    // 2. A source with brackets: exactly one allocation, the `pair: Vec<u32>`.
    let before = count();
    let mut lexer = DeepLexer::new(&js, Language::JavaScript);
    let after_new = count();
    assert_eq!(
        after_new - before,
        1,
        "one Vec for bracket pairs, built once in one scan"
    );
    let mut n = 0usize;
    while lexer.next_token(0).is_some() {
        n += 1;
    }
    let after_lex = count();
    assert!(n > 10_000, "the loop really lexed the file: {n} tokens");
    assert_eq!(
        after_lex - after_new,
        0,
        "lexing {n} tokens allocated nothing"
    );

    // 3. Python layout, strings, comments, scanf-free spans: same story.
    let before = count();
    let mut lexer = DeepLexer::new(&py, Language::Python);
    let after_new = count();
    assert_eq!(after_new - before, 1);
    let mut n = 0usize;
    while lexer.next_token(0).is_some() {
        n += 1;
    }
    assert_eq!(count() - after_new, 0, "Python layout added no allocation");
    assert!(n > 30, "Python fixture produced {n} tokens");

    // 4. The token text is borrowed from the source, not owned.
    let before = count();
    let mut lexer = DeepLexer::new(&js, Language::JavaScript);
    let mut bytes = 0usize;
    while let Some(token) = lexer.next_token(0) {
        bytes += lexer.text(&token).len();
    }
    assert!(bytes > 0);
    // One allocation again: the bracket Vec. Nothing for the text.
    assert_eq!(count() - before, 1);
}
