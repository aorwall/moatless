#!/usr/bin/env bash
# Compare the Claude catalog in `crates/types/src/model.rs` with the model each
# alias should name today.
#
#   scripts/claude-model-probe.sh
#
# `ClaudeModel::api_id` pins every alias to one model id, so nothing moves an
# alias to a newer model except an edit there. The authority for what `opus`,
# `sonnet`, `haiku` and `fable` should name is upstream T3's model manifest:
# its current Claude model whose `aliases` holds the bare alias. Each line is
#
#   alias  ours  latest  name  minCli  routable
#
# `minCli` is the Claude Code version the model needs, which the pinned
# `CLAUDE_CLI_VERSION` must reach; `routable` is whether the proxy at
# `ANTHROPIC_BASE_URL` lists it. The Anthropic pricing rows for each `latest`
# name follow, beside the price the catalog charges now.

set -euo pipefail

MANIFEST_URL=https://raw.githubusercontent.com/pingdotgg/t3code/main/apps/server/src/provider/model-manifest.json
PRICING_URL=https://platform.claude.com/docs/en/about-claude/pricing.md

cd "$(git rev-parse --show-toplevel)"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 1
fi

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

MODEL_RS=crates/types/src/model.rs

# `Self::Variant => "value"` pairs from the first `fn $1` in model.rs.
arms() {
  awk -v fn="fn $1(" '
    index($0, fn) && !done { inside = 1; next }
    inside && /^    }/ { inside = 0; done = 1 }
    inside && /Self::[A-Za-z]+ => / {
      sub(/^ *Self::/, ""); sub(/,$/, ""); sub(/ => /, "\t"); print
    }' "$MODEL_RS"
}

arms alias | sort > "$WORK/alias.tsv"
arms api_id | sort > "$WORK/api_id.tsv"
arms price | sort > "$WORK/price.tsv"
if [ ! -s "$WORK/alias.tsv" ] || [ ! -s "$WORK/api_id.tsv" ]; then
  echo "could not read ClaudeModel::alias / api_id out of $MODEL_RS" >&2
  exit 1
fi

pinned=$(sed -n 's/^CLAUDE_CLI_VERSION=//p' docker/cli-versions.env)

if ! curl -fsS -m 30 "$MANIFEST_URL" -o "$WORK/manifest.json"; then
  echo "could not fetch $MANIFEST_URL" >&2
  exit 1
fi

routable_ids=""
if [ -n "${ANTHROPIC_BASE_URL:-}" ] && [ -n "${ANTHROPIC_API_KEY:-}" ]; then
  routable_ids=$(curl -fsS -m 30 "$ANTHROPIC_BASE_URL/v1/models?limit=1000" \
    -H "x-api-key: $ANTHROPIC_API_KEY" -H "anthropic-version: 2023-06-01" \
    | jq -r '.data[].id' 2>/dev/null || true)
fi

echo "== catalog vs upstream T3 (pinned claude-code $pinned)"
printf 'alias\tours\tlatest\tname\tminCli\troutable\n'
: > "$WORK/changes"
: > "$WORK/names"
while IFS=$'\t' read -r variant alias; do
  alias=${alias//\"/}
  ours=$(awk -F'\t' -v v="$variant" '$1 == v { gsub(/"/, "", $2); print $2 }' "$WORK/api_id.tsv")
  read -r latest name min < <(jq -r --arg a "$alias" '
      [.providers.claudeAgent.models[]
       | select(.status == "current" and ((.aliases // []) | index($a)))][0]
      | if . == null then "? ? ?"
        else [.slug, (.name | gsub(" "; "_")), (.adapter.claudeCode.minVersion // "-")] | join(" ")
        end' "$WORK/manifest.json")
  name=${name//_/ }

  if [ -z "$routable_ids" ]; then
    routable="?"
  elif grep -qxF "$latest" <<<"$routable_ids"; then
    routable=yes
  else
    routable=no
  fi
  if [ "$min" != "-" ] && [ "$min" != "?" ] \
    && [ "$(printf '%s\n%s\n' "$min" "$pinned" | sort -V | head -1)" != "$min" ]; then
    min="$min(>pin)"
  fi

  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$alias" "$ours" "$latest" "$name" "$min" "$routable"
  if [ "$latest" = "?" ]; then
    echo "$alias: no current model in the manifest" >> "$WORK/changes"
  elif [ "$latest" != "$ours" ]; then
    echo "$alias: $ours -> $latest" >> "$WORK/changes"
  fi
  [ "$name" != "?" ] && printf '%s\t%s\n' "$variant" "$name" >> "$WORK/names"
done < "$WORK/alias.tsv"

echo
echo "== $PRICING_URL (input, 5m write, 1h write, cache read, output)"
if curl -fsS -m 30 "$PRICING_URL" -o "$WORK/pricing.md"; then
  # Only the first table, "Model pricing": later ones repeat the names at batch rates.
  awk '/^## Model pricing/ { on = 1; next } on && /^## / { exit } on' "$WORK/pricing.md" \
    > "$WORK/model-pricing.md"
  while IFS=$'\t' read -r variant name; do
    ours=$(awk -F'\t' -v v="$variant" '$1 == v { print $2 }' "$WORK/price.tsv")
    echo "$name — catalog: ${ours:-?}"
    grep -F "| $name " "$WORK/model-pricing.md" | sed 's/  */ /g' || echo "  (no pricing row)"
  done < "$WORK/names"
else
  echo "(could not fetch the pricing page)"
fi

echo
echo "== changes"
if [ -s "$WORK/changes" ]; then
  cat "$WORK/changes"
else
  echo "(no catalog change)"
fi
