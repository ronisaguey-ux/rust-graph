# Build every target and report the failures.
set targets {core parser cli}
set failures {}
set jobs [expr {[info exists ::env(JOBS)] ? $::env(JOBS) : 4}]

proc build {target jobs} {
    puts "building $target with $jobs jobs"
    set result [catch {exec make -C "crates/$target" -j $jobs} output]
    if {$result != 0} {
        return -code error "build of $target failed: $output"
    }
    return ok
}

foreach target $targets {
    if {[catch {build $target $jobs} problem]} {
        lappend failures $target
        puts stderr $problem
    }
}

if {[llength $failures] > 0} {
    puts stderr "failed: $failures"
    exit 1
}
puts "all targets built"
