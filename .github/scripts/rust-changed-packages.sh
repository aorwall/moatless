#!/usr/bin/env bash
# Print the `-p <pkg>` arguments for the Rust test job, narrowed to the reverse
# dependency closure of the workspace crates a PR touched.
#
# Fails safe: prints the full package list — more than necessary, never less —
# when a manifest/lockfile/toolchain/CI-tooling file changed, when a path maps
# to no workspace member, or when `cargo tree` cannot resolve the graph. A
# package leaves the list only by being provably unreachable.
#
# Usage: rust-changed-packages.sh <changed-files-file> [<pkg>...]
#   <changed-files-file>  newline-separated repo-relative paths
#   <pkg>...              the packages the caller would otherwise test. When
#                         omitted, defaults to EVERY workspace member — so a new
#                         crate is gated the day it lands, with no list to keep
#                         in sync (mirrors `cargo clippy --workspace`). Pass an
#                         explicit set only to narrow deliberately.

set -euo pipefail

CHANGED_FILES_FILE="$1"
shift
ALL_PACKAGES=("$@")

emit_all() {
    local reason="$1"
    echo "::notice::Testing all packages (${reason})" >&2
    printf -- '-p %s ' "${ALL_PACKAGES[@]}"
    echo
    exit 0
}

# Discover members from the root manifest rather than hardcoding them, so a new
# crate does not silently inherit stale routing. Done up front so the default
# package set and every fail-safe below name the full workspace.
mapfile -t MEMBER_DIRS < <(
    sed -n '/^\[workspace\]/,/^\]/p' Cargo.toml |
        sed -n 's/^[[:space:]]*"\([^"]*\)".*/\1/p'
)

if [ "${#MEMBER_DIRS[@]}" -eq 0 ]; then
    emit_all "could not parse workspace members"
fi

# Read each member directory's package name once, up front.
declare -A DIR_NAME=()
MEMBER_NAMES=()
for dir in "${MEMBER_DIRS[@]}"; do
    name=$(sed -n 's/^name = "\(.*\)"$/\1/p' "$dir/Cargo.toml" | head -1)
    if [ -z "$name" ]; then
        emit_all "could not read package name from $dir/Cargo.toml"
    fi
    DIR_NAME["$dir"]="$name"
    MEMBER_NAMES+=("$name")
done

# No explicit package arguments -> gate the entire workspace. This is the whole
# point: the caller stops maintaining a list and a carve adds nothing to CI.
if [ "${#ALL_PACKAGES[@]}" -eq 0 ]; then
    ALL_PACKAGES=("${MEMBER_NAMES[@]}")
fi

if [ ! -s "$CHANGED_FILES_FILE" ]; then
    emit_all "no changed-file list available"
fi

# Anything that can change feature resolution or the toolchain invalidates the
# per-crate reasoning below.
if grep -qE '^(Cargo\.toml|Cargo\.lock|rust-toolchain\.toml|clippy\.toml|\.cargo/config\.toml|\.config/nextest\.toml|\.github/(workflows/ci\.yml|scripts/))' "$CHANGED_FILES_FILE"; then
    emit_all "workspace manifest, toolchain or CI tooling changed"
fi

# Map each changed path to the member directory that contains it.
changed_pkgs=()
while IFS= read -r file; do
    [ -n "$file" ] || continue
    for dir in "${MEMBER_DIRS[@]}"; do
        case "$file" in
            "$dir"/*)
                changed_pkgs+=("${DIR_NAME[$dir]}")
                ;;
        esac
    done
done < "$CHANGED_FILES_FILE"

if [ "${#changed_pkgs[@]}" -eq 0 ]; then
    emit_all "no changed file maps to a workspace crate"
fi

mapfile -t changed_pkgs < <(printf '%s\n' "${changed_pkgs[@]}" | sort -u)
echo "::notice::Changed crates: ${changed_pkgs[*]}" >&2

# `cargo tree --invert` yields every crate that depends on the given one,
# transitively — the closure a change can affect.
affected=""
for pkg in "${changed_pkgs[@]}"; do
    if ! tree=$(cargo tree --workspace --invert "$pkg" --prefix none --format '{p}' 2>/dev/null); then
        emit_all "cargo tree could not resolve '$pkg'"
    fi
    affected+=$(printf '%s\n' "$tree" | awk 'NF {print $1}')$'\n'
done

selected=()
for pkg in "${ALL_PACKAGES[@]}"; do
    if printf '%s\n' "$affected" | grep -qxF "$pkg"; then
        selected+=("$pkg")
    fi
done

if [ "${#selected[@]}" -eq 0 ]; then
    emit_all "reverse-dependency closure selected nothing"
fi

# Say out loud what is being skipped. A narrowing that reports nothing reads
# like full coverage in the job log when it is not.
for pkg in "${ALL_PACKAGES[@]}"; do
    if ! printf '%s\n' "${selected[@]}" | grep -qxF "$pkg"; then
        echo "::notice::Skipping ${pkg} — unaffected by the changed crates" >&2
    fi
done

printf -- '-p %s ' "${selected[@]}"
echo
