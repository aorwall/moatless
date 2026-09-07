#!/usr/bin/env bash
# Report what the restored Rust cache actually holds, into the job summary.
#
# The `rust-test` cache entry is 2035 MB compressed and unpacks to 7.0 GB, and
# restoring it is 28% of the clippy job and 37% of the test job. Nothing
# recorded what the 7.0 GB is. The rlib-vs-crate-name gap is the signal to
# read first: the clippy job compiles 8 distinct feature sets into one target
# dir, and each feature set leaves its own `-<hash>` copy of a dependency.
#
# Usage: measure-rust-cache.sh <profile-dir-name>   # debug | ci

set -uo pipefail

PROFILE="${1:?usage: measure-rust-cache.sh <profile-dir-name>}"
CARGO_DIR="${CARGO_HOME:-$HOME/.cargo}"
TARGET="target/${PROFILE}"

# Never fail a build over a measurement: a missing dir means a cache miss, not
# a broken job.
report() {
    echo "## Rust cache footprint (\`${TARGET}\`)"
    echo ""
    echo '```'
    du -sh "$CARGO_DIR" 2>/dev/null
    du -sh target 2>/dev/null
    du -sh "$TARGET" 2>/dev/null
    echo '```'
    echo ""

    if [ ! -d "$TARGET" ]; then
        echo "_No \`${TARGET}\` — the cache did not restore._"
        return 0
    fi

    echo "### Where it sits"
    echo ""
    echo '```'
    for sub in deps build .fingerprint examples incremental; do
        du -sh "${TARGET}/${sub}" 2>/dev/null
    done
    echo '```'
    echo ""

    if [ -d "${TARGET}/deps" ]; then
        rlibs=$(find "${TARGET}/deps" -maxdepth 1 -name '*.rlib' | wc -l | tr -d ' ')
        crates=$(find "${TARGET}/deps" -maxdepth 1 -name '*.rlib' 2>/dev/null |
            sed 's|.*/||; s/^lib//; s/-[0-9a-f]*\.rlib$//' | sort -u | wc -l | tr -d ' ')
        echo "### Feature variants"
        echo ""
        echo "| rlibs in \`deps\` | distinct crate names | copies per crate |"
        echo "| --- | --- | --- |"
        if [ "$crates" -gt 0 ]; then
            echo "| ${rlibs} | ${crates} | $(awk "BEGIN { printf \"%.1f\", ${rlibs}/${crates} }") |"
        else
            echo "| ${rlibs} | ${crates} | — |"
        fi
        echo ""
    fi

    echo "### 20 largest files"
    echo ""
    echo '```'
    find "$TARGET" -type f -exec du -k {} + 2>/dev/null |
        sort -rn | head -20 |
        awk '{ printf "%8.1f MB  %s\n", $1/1024, $2 }'
    echo '```'
}

report | tee -a "${GITHUB_STEP_SUMMARY:-/dev/null}"
