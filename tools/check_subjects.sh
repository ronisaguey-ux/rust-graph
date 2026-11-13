#!/usr/bin/env bash
# Conventional-commit subjects that fit, checked over the range about to be pushed.
#
#   tools/check_subjects.sh              # the commits this branch has not pushed
#   tools/check_subjects.sh origin/main # an explicit range
#
# Fails on a subject over 72 characters or one that is not `<type>(<scope>): text`
# with a type from the list below. The length rule is not cosmetic: the remote's
# gate rejects a long subject, and finding out at push time costs the whole commit.
set -uo pipefail

cd "$(dirname "$0")/.."
BASE=${1:-}
if [ -z "$BASE" ]; then
  BASE=$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null || echo origin/main)
fi
RANGE="$BASE..HEAD"

TYPES='feat|fix|docs|test|refactor|perf|build|ci|chore|style|revert'
status=0
bad=0
total=0

while IFS= read -r subject; do
  [ -z "$subject" ] && continue
  total=$((total + 1))
  length=${#subject}
  if [ "$length" -gt 72 ]; then
    echo "LONG   ($length) $subject"
    bad=$((bad + 1)); status=1
  fi
  if ! printf '%s' "$subject" | grep -qE "^($TYPES)(\([a-z0-9._/-]+\))?: .+"; then
    echo "SHAPE        $subject"
    bad=$((bad + 1)); status=1
  fi
done < <(git log --format='%s' "$RANGE" 2>/dev/null)

if [ "$total" -eq 0 ]; then
  echo "no commits in $RANGE"
  exit 0
fi
if [ "$status" -eq 0 ]; then
  echo "OK: $total subjects conventional and under 73 characters"
fi
exit "$status"
