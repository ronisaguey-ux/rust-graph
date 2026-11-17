#!/usr/bin/env bash
# The language gate for the integrator: a patch that keeps the four measured
# corpora flat can still cost its own language, and the integrator would keep it.
#
#   integ-lang.sh <repo> <subject> [<baseline|auto>]
#
# Prints one line: "<lang> <before>/<files> -> <after>/<files>" and exits 1 when
# the language got worse. The baseline is measured on the first call for a
# language and cached in /tmp/wave-lang-baselines.json, because a full audit of
# a language costs a minute and the same number is the reference all wave.
set -uo pipefail

REPO=${1:?repo}
SUBJECT=${2:?subject}
BASELINE_FILE=/tmp/wave-lang-baselines.json
CORPORA=${RG_CORPORA:-/tmp/opencode/sweep-local}
BIN_LOCAL="${RG_INTEG_BIN:-$REPO/target/x86_64-unknown-linux-musl/release/rust-graph}"

# The corpus directory a conventional commit's scope names. `fix(c):`, `fix(cpp):`,
# `fix(csharp):` — the scope is the language, so the gate measures exactly the
# language the patch claims to fix.
scope=$(printf '%s' "$SUBJECT" | sed -n 's/^[a-z]*[(!]\{0,1\}\([a-z0-9+_-]*\)[)!]\{0,1\}:.*/\1/p')
case "$scope" in
  c) lang=c; dir=c ;;
  cpp|objc) lang=$scope; dir=$scope ;;
  csharp) lang=csharp; dir=csharp ;;
  go|java|kotlin|scala|swift) lang=$scope; dir=$scope ;;
  python|py) lang=python; dir=python ;;
  typescript|ts) lang=typescript; dir=ts ;;
  javascript|js) lang=javascript; dir=js ;;
  rust|rs) lang=rust; dir=rust ;;
  *) echo "gate: scope '$scope' carries no corpus, skipped"; exit 0 ;;
esac

if [ ! -d "$CORPORA/$dir" ]; then
  echo "gate: no corpus at $CORPORA/$dir, skipped"
  exit 0
fi
if [ ! -x "$BIN_LOCAL" ]; then
  echo "gate: no binary at $BIN_LOCAL, skipped"
  exit 1
fi

measure() {
  RG_AUDIT=1 python3 "$REPO/tools/auto_audit.py" --binary "$BIN_LOCAL" --dir "$CORPORA/$dir" --json "$lang" \
    | python3 -c 'import json,sys; d=json.load(sys.stdin)[0]; print(d["fallbacks"], d["files"], d["losses"])'
}

read -r now_files now_total now_losses <<<"$(measure)"
before=$(python3 - "$BASELINE_FILE" "$lang" <<'PY'
import json, os, sys
path, lang = sys.argv[1], sys.argv[2]
try:
    data = json.load(open(path))
except Exception:
    data = {}
print(data.get(lang, {}).get("fallbacks", -1))
PY
)

if [ "$before" -lt 0 ]; then
  python3 - "$BASELINE_FILE" "$lang" "$now_files" "$now_total" "$now_losses" <<'PY'
import json, sys
path, lang, fb, total, losses = sys.argv[1:6]
try:
    data = json.load(open(path))
except Exception:
    data = {}
data[lang] = {"fallbacks": int(fb), "files": int(total), "losses": int(losses)}
json.dump(data, open(path, "w"), indent=1)
PY
  echo "$lang baseline $now_files/$now_total"
  exit 0
fi

total=$(python3 -c "
import json,sys
d=json.load(open('$BASELINE_FILE'))
print(d['$lang']['files'])")
echo "$lang $before/$total -> $now_files/$total"
if [ "$now_files" -gt "$before" ]; then
  echo "gate: $lang got worse ($before -> $now_files)"
  exit 1
fi
exit 0
