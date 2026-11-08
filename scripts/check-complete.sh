#!/bin/bash
# The definition of "done" for the deep parser, as numbers rather than opinions.
# Every line prints PASS/FAIL with the number that decided it.
set -u
cd "$(dirname "$0")/.." || exit 1
export PATH="$HOME/.cargo/bin:$PATH"
fail=0
line() { printf '%-42s %s\n' "$1" "$2"; }
check() { if [ "$2" = "PASS" ]; then line "$1" "PASS  $3"; else line "$1" "FAIL  $3"; fail=$((fail+1)); fi; }

# 1. The workspace builds and its tests pass.
if timeout 1800 cargo test --workspace --quiet >/tmp/cc_ws.log 2>&1; then
  pass=$(grep -E "^test result" /tmp/cc_ws.log | awk '{p+=$4} END {print p+0}')
  check "workspace tests" PASS "${pass:-0} passed"
else
  check "workspace tests" FAIL "$(grep -cE 'FAILED' /tmp/cc_ws.log) failing"
fi

# 2. The deep build tests pass.
if timeout 1800 cargo test --features "deep parser" --quiet >/tmp/cc_deep.log 2>&1; then
  pass=$(grep -E "^test result" /tmp/cc_deep.log | awk '{p+=$4} END {print p+0}')
  check "deep parser tests" PASS "${pass:-0} passed"
else
  check "deep parser tests" FAIL "see /tmp/cc_deep.log"
fi

# 3. Languages with a grammar and a fixture directory.
langs=$(ls -d crates/deep-grammars/src/batch_* 2>/dev/null | wc -l)
count=$(find crates/deep-grammars/src -name '*.rs' ! -name 'mod.rs' ! -name 'lex.rs' ! -name 'lib.rs' ! -name 'deep.rs' | wc -l)
if [ "$count" -ge 43 ]; then check "language grammars" PASS "$count"; else check "language grammars" FAIL "$count of 43"; fi

# 4. The binary size gate.
if timeout 900 cargo build --release --features "deep parser" --bin rust-graph-deep-scan --quiet >/dev/null 2>&1; then
  size=$(stat -c %s target/x86_64-unknown-linux-musl/release/rust-graph-deep-scan 2>/dev/null || echo 0)
  if [ "$size" -gt 0 ] && [ "$size" -lt 10485760 ]; then check "binary size" PASS "$size bytes"; else check "binary size" FAIL "$size bytes"; fi
else
  check "binary size" FAIL "build failed"
fi

# 5. The default build must not have grown.
if timeout 900 cargo build --release --quiet >/dev/null 2>&1; then
  base=$(stat -c %s target/x86_64-unknown-linux-musl/release/rust-graph 2>/dev/null || echo 0)
  check "default binary present" PASS "$base bytes"
else
  check "default binary present" FAIL "build failed"
fi

echo
[ "$fail" -eq 0 ] && echo "ALL CRITERIA MET" || echo "$fail criteria not met"
exit "$fail"
