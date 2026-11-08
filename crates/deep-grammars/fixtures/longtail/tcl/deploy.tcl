#!/usr/bin/env tclsh
# A small deploy helper in the Tcl style.
set ::release_dir "/srv/releases"
set ::stamp [clock format [clock seconds] -format %Y%m%d%H%M%S]

proc log {message} {
    puts stderr "\[$::stamp\] $message"
}

proc usage {} {
    puts stderr "usage: deploy.tcl <environment>"
    exit 64
}

proc fetch {url target} {
    log "fetching $url"
    set channel [open $target w]
    puts $channel "placeholder for $url"
    close $channel
    return $target
}

if {$argc < 1} {
    usage
}

set environment [lindex $argv 0]
log "deploying to $environment"

foreach host {web01 web02 web03} {
    log "preparing $host"
    set archive [fetch "https://artifacts.example.com/latest.tar.gz" "/tmp/$host.tgz"]
    puts "$host <- $archive"
}
