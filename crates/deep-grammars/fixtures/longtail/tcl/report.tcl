# Turn the scan summary into markdown.
set summary [dict create count 0 bytes 0 elapsed 0.0]

proc row {name value} {
    format "| %-16s | %10s |" $name $value
}

proc render {summary} {
    set lines [list "## Scanner summary" ""]
    lappend lines [row files [dict get $summary count]]
    lappend lines [row bytes [dict get $summary bytes]]
    return [join $lines "\n"]
}

dict set summary count 1843
dict set summary bytes 9123456
puts [render $summary]
