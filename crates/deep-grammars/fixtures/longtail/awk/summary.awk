#!/usr/bin/awk -f
# Summarise a log: counts per status and the slowest request.
BEGIN {
    FS = " "
    total = 0
    max = 0
}

$1 == "GET" || $1 == "POST" {
    total += 1
    status[$9] += 1
    if ($NF > max) {
        max = $NF
        slowest = $7
    }
}

END {
    printf "requests: %d\n", total
    for (code in status) {
        printf "  %s -> %d\n", code, status[code]
    }
    printf "slowest: %s (%d ms)\n", slowest, max
}

function label(code) {
    if (code ~ /^2/) return "ok"
    if (code ~ /^3/) return "redirect"
    if (code ~ /^4/) return "client"
    return "server"
}
