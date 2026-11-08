#!/usr/bin/env bash
# A deploy helper: the shape of most operational scripts.
set -euo pipefail

APP=webapi
RELEASE_DIR=/srv/releases
STAMP=$(date +%Y%m%d%H%M%S)

usage() {
    echo "usage: deploy.sh <environment>" >&2
    exit 64
}

log() {
    printf '[%s] %s\n' "$STAMP" "$*" >&2
}

if [ "$#" -lt 1 ]; then
    usage
fi

ENVIRONMENT=$1

case_guard() {
    if [ "$ENVIRONMENT" = "prod" ] && [ -z "${ALLOW_PROD:-}" ]; then
        log "refusing to deploy to prod without ALLOW_PROD"
        return 1
    fi
    return 0
}

fetch() {
    local url=$1
    curl --fail --silent --show-error --location "$url" -o "$RELEASE_DIR/$APP-$STAMP.tar.gz"
}

for host in web01 web02 web03
do
    log "preparing $host"
    ssh "deploy@$host" "mkdir -p $RELEASE_DIR"
done

if ! case_guard; then
    log "aborted"
    exit 1
fi

fetch "https://artifacts.example.com/$APP/latest.tar.gz" | tee deploy.log
