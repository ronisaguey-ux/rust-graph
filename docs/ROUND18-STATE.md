# Neon round 18 — integration state (2026-11-25)

Lane 1 (dart, bash, ruby) packaged `finished_mega_r18.zip`; lane 2 packaged a
byte-identical copy of its round-16 zip, so lane 2 shipped nothing this round.

## Lane 1 r18 — verified on a host build, not on the lane's word

Built from the lane tree with `--features 'deep parser'` into
`rg-kbuilds/neon18l1`, measured on the lane's own shipped corpora:

| lang | normal before | normal after | large before | large after |
|---|---|---|---|---|
| dart | 291/4,686 (6.21% file, 16.55% LOC) | 71/4,686 (1.52%, 5.61%) | 101/733 (13.78%, 21.07%) | 30/733 (4.09%, 7.24%) |
| bash | 499/1,679 (29.72%, 58.24%) | 121/1,679 (7.21%, 16.62%) | 17/20 (85.0%, 76.84%) | 10/20 (50.0%, 26.6%) |
| ruby | — | 72/1,836, 164 rows | — | (measuring) |

Every measured figure matched the lane's own report for dart and bash; ruby's
normal figure matched too (72 files, 164 rows). The lane's ruby normal run
exited rc=2 on its side; the host run is clean, so the tool is not the fault.

## Merge is prepared, build deferred

- The lane's r18 tree differs from its r17 tree in `src/deep_facts/mod.rs` only.
- 3-way merge (`git merge-file`, base = r17 lane tree, ours = this repo) is
  conflict-free and adds the `augment_dart` / `augment_shell` readers:
  **`/tmp/opencode/r18/mergedL1_mod.rs`** (+1,331 lines vs repo mod.rs).
- `src/deep_facts/go.rs` and `src/pipeline.rs` are NOT taken from the lane: the
  lane tree predates lane 2's `augment_c` / go.rs work. Only mod.rs is merged.
- The repo build was stopped on purpose: the owner's box rule is <50% RAM/CPU
  while he studies, and one rustc pass pushed used RAM past the line. Re-run
  `CARGO_TARGET_DIR=rg-kbuilds/neon18c cargo build --release --features 'deep parser' -j1`
  in a quiet window, then measure dart/bash/ruby on the repo build, then
  `stack_atomic_commit.py` + `git push ghpub HEAD:integ-45`.

## Not to do

- Never copy the lane's `shengcun` over the repo copy.
- Do not measure with a binary built without `--features 'deep parser'`: it
  reports zero losses for every language (a 1.1 MB binary instead of 11 MB).
- `.cargo/config.toml` must be present in any extracted lane tree, or the build
  dies on `linker cc not found`.
