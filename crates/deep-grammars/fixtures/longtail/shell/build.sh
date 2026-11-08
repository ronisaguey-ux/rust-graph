#!/bin/bash
# Build and test, with a subshell per target.
targets=(core parser cli)

function build_target {
    local target=$1
    local jobs=${JOBS:-4}
    echo "building $target with $jobs jobs"
    make -C "crates/$target" -j "$jobs" >> build.log 2>&1
}

function test_target {
    local target=$1
    cargo test --quiet -p "$target" || {
        echo "tests failed for $target" >&2
        return 1
    }
}

for target in "${targets[@]}"; do
    build_target "$target" && test_target "$target"
done

(
    cd docs && make html
) &

wait

[ -f build.log ] && tail -n 20 build.log
echo "all targets built"
