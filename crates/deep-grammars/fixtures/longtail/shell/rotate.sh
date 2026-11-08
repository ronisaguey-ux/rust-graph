#!/bin/sh
# Rotate logs while there is still disk space.
LOG_DIR=/var/log/app
MAX_AGE=14
dry_run=0

if [ "${1:-}" = "--dry-run" ]; then
    dry_run=1
fi

find "$LOG_DIR" -name '*.log' -mtime +$MAX_AGE -print > /tmp/rotate.list

while read -r path; do
    echo "considering $path"
    if [ "$dry_run" -eq 0 ]; then
        gzip -9 "$path"
    fi
done < /tmp/rotate.list

until [ "$(du -sm "$LOG_DIR" | cut -f1)" -le 512 ]; do
    oldest=oldest.log
    rm -- "$LOG_DIR/$oldest"
done

printf 'done at %s\n' "$(date -Iseconds)"
exit 0
