#!/bin/sh
# Builds target/bench/tsdrv, the Tree-sitter side of the parser gate, from
# pinned sources, and fetches the pinned repositories the corpora come from. Run from the repository root:
#
#     sh bench/build-tsdrv.sh
#
# Sources are fetched once into target/bench/src (one shallow fetch per pinned
# commit) and reused. TSDRV_SRC=DIR uses checkouts already in DIR instead; each
# must still be at its pinned commit, or the build stops.
set -eu

ROOT=$(pwd)
OUT="$ROOT/target/bench"
SRC="${TSDRV_SRC:-$OUT/src}"
CC="${CC:-cc}"
mkdir -p "$OUT/obj" "$SRC"

# name  repository  commit  grammar directories (comma separated; - for none)
# tree-sitter itself is the v0.27.2 release. Rows with no grammar are corpus
# sources, named by the lists in bench/corpus as target/bench/src/NAME/...:
# rust-analyzer is its 2026-11-02 release tag, tokio its tokio-1.54.1 tag,
# webpack its v5.112.2 tag, vue-core v3.5.45 and excalidraw v0.18.1.
PINS='
tree-sitter            tree-sitter/tree-sitter            3fb7be49145acfb07e33b61ed5689e2b6771b2e9 -
tree-sitter-python     tree-sitter/tree-sitter-python     26855eabccb19c6abf499fbc5b8dc7cc9ab8bc64 .
tree-sitter-rust       tree-sitter/tree-sitter-rust       77a3747266f4d621d0757825e6b11edcbf991ca5 .
tree-sitter-javascript tree-sitter/tree-sitter-javascript 58404d8cf191d69f2674a8fd507bd5776f46cb11 .
tree-sitter-typescript tree-sitter/tree-sitter-typescript 75b3874edb2dc714fb1fd77a32013d0f8699989f typescript,tsx
tree-sitter-go         tree-sitter/tree-sitter-go         2346a3ab1bb3857b48b29d779a1ef9799a248cd7 .
tree-sitter-java       tree-sitter/tree-sitter-java       e10607b45ff745f5f876bfa3e94fbcc6b44bdc11 .
tree-sitter-c-sharp    tree-sitter/tree-sitter-c-sharp    8c0abe0b84a3681d3e6852ce2e2a1eecb4f731c5 .
tree-sitter-php        tree-sitter/tree-sitter-php        58d5643086ae60e93a3746cca648363ee97c8761 php
tree-sitter-ruby       tree-sitter/tree-sitter-ruby       ad907a69da0c8a4f7a943a7fe012712208da6dee .
tree-sitter-c          tree-sitter/tree-sitter-c          b780e47fc780ddc8da13afa35a3f4ed5c157823d .
tree-sitter-cpp        tree-sitter/tree-sitter-cpp        c009222808634c1014f82438d4883753516a2c24 .
tree-sitter-json       tree-sitter/tree-sitter-json       254c42a6476413b776221e03982ac8ae159eeb72 .
rust-analyzer          rust-lang/rust-analyzer            b98172d80f0da12bbde3d6733c274e17cb6417aa -
tokio                  tokio-rs/tokio                     b53b2001ed2ccfd9b87e32a0c6da1a1eebdc80b8 -
webpack                webpack/webpack                    e95a7f2764281318c038e9384371797dfef0e48a -
vue-core               vuejs/core                         8b479a89445868d474f17e2d837765e7c9ece5e4 -
excalidraw             excalidraw/excalidraw              a2ec2889babf7d2295469c6d90ebe77fae57df84 -
'

fetch() { # name repository commit
    dir="$SRC/$1"
    if [ ! -d "$dir/.git" ]; then
        if [ -n "${TSDRV_SRC:-}" ]; then
            echo "build-tsdrv: $dir is missing (TSDRV_SRC=$TSDRV_SRC)" >&2
            exit 1
        fi
        git init -q "$dir"
        git -C "$dir" fetch -q --depth 1 "https://github.com/$2" "$3"
        git -C "$dir" -c advice.detachedHead=false checkout -q FETCH_HEAD
    fi
    have=$(git -C "$dir" rev-parse HEAD)
    if [ "$have" != "$3" ]; then
        echo "build-tsdrv: $dir is at $have, pinned $3" >&2
        exit 1
    fi
}

echo "$PINS" | while read -r name repository commit grammars; do
    if [ -n "$name" ]; then fetch "$name" "$repository" "$commit"; fi
done

compile() { # source object include...
    source=$1 object=$2
    shift 2
    if [ ! -f "$object" ] || [ "$source" -nt "$object" ]; then
        "$CC" -O3 -std=gnu11 -fPIC -w "$@" -c "$source" -o "$object"
    fi
}

TS="$SRC/tree-sitter/lib"
compile "$TS/src/lib.c" "$OUT/obj/lib.o" -I"$TS/include" -I"$TS/src"
OBJS="$OUT/obj/lib.o"
for line in $(echo "$PINS" | awk '$4 != "-" && NF { print $1 ":" $4 }'); do
    name=${line%%:*}
    for sub in $(echo "${line#*:}" | tr ',' ' '); do
        dir="$SRC/$name/$sub/src"
        tag=$(echo "$name-$sub" | tr '/.' '__')
        compile "$dir/parser.c" "$OUT/obj/$tag-parser.o" -I"$dir"
        OBJS="$OBJS $OUT/obj/$tag-parser.o"
        if [ -f "$dir/scanner.c" ]; then
            compile "$dir/scanner.c" "$OUT/obj/$tag-scanner.o" -I"$dir"
            OBJS="$OBJS $OUT/obj/$tag-scanner.o"
        fi
    done
done

"$CC" -O2 -std=gnu11 -I"$TS/include" "$ROOT/bench/tsdrv.c" $OBJS -rdynamic -ldl -o "$OUT/tsdrv"
echo "build-tsdrv: built $OUT/tsdrv"
