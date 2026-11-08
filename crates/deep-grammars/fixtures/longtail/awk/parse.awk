function trim(text) {
    gsub(/^[ \t]+|[ \t]+$/, "", text)
    return text
}

function split_line(line, parts,   n) {
    n = split(line, parts, "|")
    return n
}

/^#/ {
    next
}

NF >= 3 {
    name = trim($1)
    value = trim($2)
    if (value == "") {
        printf "empty value for %s\n", name
    } else {
        pairs[name] = value
    }
}

END {
    for (key in pairs) {
        print key "=" pairs[key]
    }
}
