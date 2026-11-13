#!/usr/bin/env bash
# One Vast sweep: rent an 80-core box, run the corpus sweep, destroy it — and
# destroy it even when the sweep dies. A rented box that outlives its job is the
# only way this harness can cost more than the four cents a sweep is budgeted
# at, so the destroy lives in a trap rather than at the end of the script.
#
#   tools/vast_sweep.sh --disk 80 --image "<image>" -- sweep command...
#
# --self-test proves the trap with a fake vastai on PATH and a sweeper that
# exits nonzero: if destroy is not called, the test fails.
set -uo pipefail

VASTAI=${VASTAI:-vastai}
BUDGET_CENTS=${BUDGET_CENTS:-4}
INSTANCE=""
DESTROYED=0

destroy() {
  local status=$?
  if [ -n "$INSTANCE" ] && [ "$DESTROYED" -eq 0 ]; then
    DESTROYED=1
    # `destroy` is retried once: a transient API error must not leave the box
    # running, and the failure has to be loud because it is money.
    if ! "$VASTAI" destroy instance "$INSTANCE" --raw >/dev/null 2>&1; then
      sleep 3
      if ! "$VASTAI" destroy instance "$INSTANCE" --raw >/dev/null 2>&1; then
        echo "DESTROY FAILED for instance $INSTANCE — destroy it by hand" >&2
        exit 3
      fi
    fi
  fi
  exit "$status"
}
trap destroy EXIT INT TERM HUP

rent() {
  local raw
  raw=$("$VASTAI" create instance "$@" --raw) || return 1
  INSTANCE=$(printf '%s' "$raw" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("new_contract",""))')
  [ -n "$INSTANCE" ] || { echo "rent returned no contract id" >&2; return 1; }
  echo "instance $INSTANCE rented (budget ${BUDGET_CENTS}c)"
}

self_test() {
  local tmp fake
  tmp=$(mktemp -d)
  fake="$tmp/vastai"
  cat >"$fake" <<'EOF'
#!/usr/bin/env bash
echo "$*" >>"$VASTAI_LOG"
case "$1" in
  create) echo '{"new_contract": 12345}' ;;
  destroy) echo '{}' ;;
esac
EOF
  chmod +x "$fake"
  : >"$tmp/log"
  VASTAI_LOG="$tmp/log" VASTAI="$fake" bash -c '
    set -uo pipefail
    source "$1" --source-only 2>/dev/null || true
  ' _ "$0" >/dev/null 2>&1

  # Run this same script with the fake renting and a sweeper that fails.
  VASTAI_LOG="$tmp/log" VASTAI="$fake" SWEEP_CMD='exit 1' "$0" --no-rent-test >/dev/null 2>&1
  local rc=$?
  if ! grep -q '^destroy instance 12345' "$tmp/log"; then
    echo "SELF-TEST FAILED: destroy did not run after a failed sweep (rc=$rc)" >&2
    cat "$tmp/log" >&2
    rm -rf "$tmp"
    return 1
  fi
  echo "SELF-TEST OK: failed sweep still destroyed instance 12345"
  rm -rf "$tmp"
  return 0
}

main() {
  local args=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --self-test) self_test; exit $? ;;
      --no-rent-test) INSTANCE=12345; eval "${SWEEP_CMD:-exit 1}"; return ;;
      --) shift; break ;;
      *) args+=("$1"); shift ;;
    esac
  done

  rent "${args[@]}" || { echo "rent failed; nothing to destroy" >&2; exit 2; }
  if [ $# -eq 0 ]; then
    echo "no sweep command given" >&2
    exit 2
  fi
  "$@"
}

main "$@"
