#!/bin/bash
# The before/after pair every pairing change carries in its commit body.
#
#   tools/sweep_pair.sh before   # on the parent commit, before the change
#   tools/sweep_pair.sh after    # on the change
#
# Both runs use the same binary and the same corpora, and the output is one line
# per corpus so it pastes into a commit message without editing. A change is only
# landable when the "after" line falls where it claims and nothing else rises;
# a rise on a corpus the change does not name is a revert, not a note.
#
# The binary is the release build: the debug one takes minutes per corpus and
# measures nothing the release build does not.
set -u

which=${1:-after}
root=${RG_CORPORA:-/tmp/opencode/sweep-local}
bin=${RG_BIN:-/home/roni/Roni_workspace/rg-kbuilds/release/x86_64-unknown-linux-musl/release/rust-graph}

if [ ! -x "$bin" ]; then
    echo "no binary at $bin (build it, or set RG_BIN)" >&2
    exit 1
fi

for lang in js ts python rust; do
    dir="$root/$lang"
    [ -d "$dir" ] || { echo "$lang missing" >&2; continue; }
    out=$(mktemp)
    "$bin" build "$dir" --no-cache --stdout > "$out" 2>/dev/null
    printf '%s %s ' "$which" "$lang"
    python3 - "$out" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
rows = data['sources']
fallbacks = sum(1 for row in rows if row.get('deep_fallback'))
print(f"{fallbacks}/{len(rows)}")
PY
    rm -f "$out"
done
