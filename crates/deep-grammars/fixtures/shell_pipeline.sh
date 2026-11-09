#!/usr/bin/env bash
# A shaped-like-production deploy script: functions, guards, pipelines.
set -euo pipefail
IFS=$'\n\t'

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly LOG_FILE="${LOG_FILE:-/var/log/deploy.log}"
readonly MAX_RETRIES="${MAX_RETRIES:-3}"

log() {
	local level="$1"
	shift
	printf '%s [%s] %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$level" "$*" |
		tee -a "$LOG_FILE" >&2
}

die() {
	log error "$*"
	exit 1
}

retry() {
	local attempt=1
	until "$@"; do
		if ((attempt >= MAX_RETRIES)); then
			return 1
		fi
		log warn "retrying ($attempt/$MAX_RETRIES): $*"
		sleep "$((attempt * 2))"
		attempt=$((attempt + 1))
	done
}

deploy_unit() {
	local unit="$1"
	local target="${2:-production}"

	if ! systemctl is-active --quiet "$unit"; then
		die "unit $unit is not running"
	fi

	retry curl --silent --fail --max-time 10 "https://deploy.example.com/$target/$unit" \
		--header "Authorization: Bearer $TOKEN" \
		--data-binary @- <<'PAYLOAD' >/dev/null
{"unit": "$unit"}
PAYLOAD

	log info "deployed $unit to $target"
}

main() {
	local -a units
	mapfile -t units < <(find "$SCRIPT_DIR/units" -name '*.service' -printf '%f\n')

	if [[ ${#units[@]} -eq 0 ]]; then
		die 'no units to deploy'
	fi

	for unit in "${units[@]}"; do
		case "$unit" in
			*.service) deploy_unit "${unit%.service}" ;;
			skip-*) log debug "skipping $unit" ;;
			*) log warn "unexpected unit name: $unit" ;;
		esac
	done

	log info "done: ${#units[@]} units"
}

main "$@"
