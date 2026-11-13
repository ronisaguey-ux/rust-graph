# The arena: what it is, and why it is not the layout the plan asked for

The tree is one preorder `Vec<Node>` of 16-byte records — `kind`, `flags`, byte
`start`, byte `end`, and `subtree_end` — with `u32` indices. `subtree_end` is the
index one past a node's last descendant, so a subtree is a contiguous slice and
skipping one is a single load. Every walk in `src/deep/tree.rs` is a loop; none
recurses, so depth cannot overflow the stack.

## What P3-01 asked for, and what it would cost

The item asks for a struct-of-arrays with kind, parent, first child, next
sibling, byte range and line, sized once per file from a token-count estimate.

| field | bytes | needed by the walks today |
| --- | ---: | --- |
| `kind` | 2 | yes |
| `flags` | 2 | yes (`HAS_ERROR`, `UNCLOSED`, `MISSING`, `HAS_SKIP`, `TOKEN`) |
| `start`, `end` | 8 | yes |
| `first_child`, `next_sibling` | 8 | **no** — `children()` derives both from `subtree_end` |
| `parent` | 4 | **no** — no walk asks for a parent; the parser has the stack |
| `line` | 4 | **no** — `line_of` is a binary search over line starts, on demand |

So the proposed record is 24–28 bytes against today's 16: a 50–75% larger arena,
carrying three fields nothing reads, in exchange for losing the contiguous-subtree
property that makes skipping a subtree one load. A struct-of-arrays would not
shrink the record either — the same six fields in six vectors cost the same bytes;
it changes the access pattern, and the walks here are preorder, which is exactly
what an array of records is good at.

Measured 2026-11-13 on the 1,995-file JavaScript corpus, release build, cold
cache: peak RSS 63.6 MB before, 63.7 MB after the reserve below, wall 14.3–17.7 s
across two runs (the run-to-run spread is larger than any difference here). There
is no measurement in hand that a different layout would improve.

## What did land

`build()` now sizes the arena once from the token count instead of growing it:

```rust
let estimate = tokens.kind.len().saturating_mul(2).saturating_add(16);
if self.nodes.capacity() < estimate {
    self.nodes.reserve(estimate - self.nodes.len());
}
```

A node per token plus the groups that wrap them, with slack for the wrappers a
grammar inserts. The allocation count is deterministic rather than amortised,
which is the part of the item that is a real property of the build; the rest of
it is a layout the numbers do not support.

## When to revisit

The layout becomes the right question if a walk ever needs a parent link — an
incremental reparse that has to walk up, or an edit that must find the enclosing
node by index rather than by stack. At that point the cost is a fourth field and
the benefit is a walk that does not rescan. Until then this is a 16-byte record
that every consumer already reads in the order it is stored.
