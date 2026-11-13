#!/usr/bin/env bash
# Measure all four subsets, not just the one the change was written for.
#
# A JS-only win that costs TypeScript a row is a regression the totals do not
# show, so every measurement runs js, ts, python and rust against their own
# baselines and the whole run fails if any subset lost a row.
#
#   tools/measure_subsets.sh                 # check against the committed baselines
#   tools/measure_subsets.sh --update js ts  # re-baseline named subsets
#
# The corpora default to /tmp/sub_<lang> built from the first fifty files of
# each local corpus; override with SUBSET_ROOT=/somewhere.
set -uo pipefail

cd "$(dirname "$0")/.."
HERE=$(pwd)
ROOT=${SUBSET_ROOT:-/tmp}
UPDATE=0
LANGS=()

for arg in "$@"; do
  case "$arg" in
    --update) UPDATE=1 ;;
    js|ts|python|rust) LANGS+=("$arg") ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done
[ ${#LANGS[@]} -eq 0 ] && LANGS=(js ts python rust)

if [ -z "${RUST_GRAPH_BIN:-}" ]; then
  RUST_GRAPH_BIN=$(command -v rust-graph || true)
fi
if [ -z "$RUST_GRAPH_BIN" ] || [ ! -x "$RUST_GRAPH_BIN" ]; then
  echo "set RUST_GRAPH_BIN to the binary you just built" >&2
  exit 2
fi
export RUST_GRAPH_BIN

status=0
for lang in "${LANGS[@]}"; do
  corpus="$ROOT/sub_$lang"
  [ "$lang" = js ] && corpus="$ROOT/js50"
  baseline="$HERE/tools/baselines/event_${lang}50.json"
  if [ ! -d "$corpus" ]; then
    echo "SKIP $lang: no corpus at $corpus" >&2
    status=1
    continue
  fi
  if [ "$UPDATE" -eq 1 ]; then
    printf '%-7s ' "$lang"
    ./tools/event_regression.py --corpus "$corpus" --baseline "$baseline" --update 2>&1 | tail -1
    continue
  fi
  printf '%-7s ' "$lang"
  if ! ./tools/event_regression.py --corpus "$corpus" --baseline "$baseline" 2>&1 | tail -1; then
    status=1
  fi
done

if [ "$status" -ne 0 ]; then
  echo "MEASURE FAILED: a subset lost a row or could not be measured"
fi
exit "$status"
