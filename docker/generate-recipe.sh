#!/bin/bash
# docker/generate-recipe.sh — recipe (docker/recipes/<name>.json) -> Dockerfile / content-hash tag.
#
# A recipe is {"base": "sandbox"|"sandbox-base", "features": {<id>: {<option>: <value>, ...}}}.
# Single source of truth for build-sandboxes.yml's discover and build
# steps, so a tag computed by one always matches the other.
#
# The tag folds in more than the literal recipe bytes on purpose:
#   - the resolved digest of <base>:latest, so a sandbox/sandbox-base rebuild
#     produces a new tag instead of being silently skipped as "already built";
#   - a content hash of each referenced feature's files, so an install.sh edit
#     (e.g. a CVE patch) does the same.
#
# A feature's optional feature.json "env" map is emitted as real Dockerfile
# ENV instructions right after that feature's RUN step — install.sh can't set
# ENV itself (it runs inside a RUN'd shell), and the sandbox entrypoint execs
# the sandbox binary directly without sourcing /etc/profile, so ENV is the
# only thing the running process actually inherits.
#
# Usage:
#   docker/generate-recipe.sh tag <recipe.json>
#   docker/generate-recipe.sh dockerfile <recipe.json> [outfile]

set -euo pipefail

REGISTRY="${REGISTRY_ORG:-aorwall}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
FEATURES_DIR="$REPO_ROOT/docker/features"

die() { echo "error: $*" >&2; exit 1; }

resolve_base_repo() {
  case "$1" in
    sandbox)      echo "${REGISTRY}/moatless-sandbox" ;;
    sandbox-base) echo "${REGISTRY}/moatless-sandbox-base" ;;
    *) die "unknown recipe base '$1' (expected 'sandbox' or 'sandbox-base')" ;;
  esac
}

resolve_base_digest() {
  local ref="$1" digest
  digest="$(docker buildx imagetools inspect "$ref" 2>/dev/null | awk '/^Digest:/{print $2; exit}')"
  [ -n "$digest" ] || die "could not resolve digest for $ref"
  echo "$digest"
}

feature_content_hash() {
  local id="$1" dir="$FEATURES_DIR/$id"
  [ -d "$dir" ] || die "unknown feature '$id' (no $dir)"
  ( cd "$dir" && find . -type f | LC_ALL=C sort | xargs -r sha256sum ) | sha256sum | awk '{print $1}'
}

# Deterministic order: repeatedly pick the alphabetically-first remaining id
# whose installsAfter deps (that are actually part of this recipe) are all
# already ordered.
order_features() {
  local recipe="$1" ids nodes="[]" id fjson installs_after

  ids="$(jq -r '.features | keys[]' "$recipe" | LC_ALL=C sort)"
  while IFS= read -r id; do
    [ -n "$id" ] || continue
    fjson="$FEATURES_DIR/$id/feature.json"
    [ -f "$fjson" ] || die "recipe references unknown feature '$id' (no $fjson)"
    installs_after="$(jq -c '.installsAfter // []' "$fjson")"
    nodes="$(jq -c --arg id "$id" --argjson ia "$installs_after" \
      '. + [{id: $id, installsAfter: $ia}]' <<<"$nodes")"
  done <<<"$ids"

  jq -r '
    (map(.id) | sort) as $ids
    | (map({(.id): ([ .installsAfter[] | select(. as $d | $ids | index($d) != null) ]) }) | add) as $deps
    | reduce range(0; ($ids|length)) as $i
        ( {ordered: [], remaining: $ids};
          . as $state
          | ($state.remaining
              | map(select( (($deps[.] // []) - $state.ordered) | length == 0 ))
              | sort) as $ready
          | if ($ready|length) == 0 then
              error("installsAfter cycle among: " + ($state.remaining | join(",")))
            else
              { ordered: ($state.ordered + [$ready[0]]),
                remaining: ($state.remaining - [$ready[0]]) }
            end
        )
    | .ordered[]
  ' <<<"$nodes"
}

cmd_tag() {
  local recipe="$1" canonical base base_repo base_digest ids id fh combined
  canonical="$(jq -S -c . "$recipe")"
  base="$(jq -r '.base' "$recipe")"
  base_repo="$(resolve_base_repo "$base")"
  base_digest="$(resolve_base_digest "${base_repo}:latest")"

  combined="base:${base_digest}
recipe:${canonical}"
  ids="$(jq -r '.features | keys[]' "$recipe" | LC_ALL=C sort)"
  while IFS= read -r id; do
    [ -n "$id" ] || continue
    fh="$(feature_content_hash "$id")"
    combined="${combined}
feature:${id}:${fh}"
  done <<<"$ids"

  printf '%s' "$combined" | sha256sum | cut -c1-12
}

cmd_dockerfile() {
  local recipe="$1" out="${2:-/dev/stdout}" base base_repo base_digest tag id opts
  base="$(jq -r '.base' "$recipe")"
  base_repo="$(resolve_base_repo "$base")"
  base_digest="$(resolve_base_digest "${base_repo}:latest")"
  tag="$(cmd_tag "$recipe")"

  {
    echo "# GENERATED FILE — do not edit by hand."
    echo "# Source recipe: $recipe"
    echo "# Content-hash tag: $tag"
    echo "# Regenerate: docker/generate-recipe.sh dockerfile $recipe"
    echo "FROM ${base_repo}@${base_digest}"
    echo
    for id in $(order_features "$recipe"); do
      opts="$(jq -r --arg id "$id" '
        (.features[$id] // {})
        | to_entries
        | map("\(($id|ascii_upcase))_\((.key|ascii_upcase))=\(.value|tostring)")
        | join(" ")
      ' "$recipe")"
      echo "COPY docker/features/${id}/ /tmp/features/${id}/"
      if [ -n "$opts" ]; then
        echo "RUN ${opts} /tmp/features/${id}/install.sh && rm -rf /tmp/features/${id}"
      else
        echo "RUN /tmp/features/${id}/install.sh && rm -rf /tmp/features/${id}"
      fi
      while IFS=$'\t' read -r env_key env_val; do
        [ -n "$env_key" ] || continue
        echo "ENV ${env_key}=\"${env_val}\""
      done < <(jq -r '(.env // {}) | to_entries[] | [.key, .value] | @tsv' "$FEATURES_DIR/$id/feature.json")
    done
  } > "$out"
}

main() {
  command -v jq >/dev/null 2>&1 || die "jq is required"
  command -v docker >/dev/null 2>&1 || die "docker is required (buildx imagetools inspect)"
  case "${1:-}" in
    tag)        [ $# -eq 2 ] || die "usage: $0 tag <recipe.json>"; cmd_tag "$2" ;;
    dockerfile) [ $# -ge 2 ] || die "usage: $0 dockerfile <recipe.json> [outfile]"; cmd_dockerfile "$2" "${3:-/dev/stdout}" ;;
    *) die "usage: $0 {tag|dockerfile} <recipe.json> [outfile]" ;;
  esac
}

main "$@"
