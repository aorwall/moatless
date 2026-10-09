#!/usr/bin/env bash
# Regenerate the T3 HTML render bootstrap an inline Artifact is served with, and
# the theme variable list `moat artifact publish --help` gives the agent.
#
# The `<style>` and `<script>` come from `injectHtmlRenderBootstrap`, and the
# list from `HTML_RENDER_THEME_GUIDE`, in t3code's
# `packages/shared/src/htmlRender.ts`. Run this whenever a t3code upstream merge
# changes that file: a stale copy no longer fits the frame to the page or
# follows the theme, and nothing reports an error.
#
# Usage: scripts/vendor-html-render-bootstrap.sh <path to a t3code checkout>

set -euo pipefail

T3CODE_DIR="${1:?usage: $0 <path to a t3code checkout>}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOURCE="$(cd "$T3CODE_DIR" && pwd)/packages/shared/src/htmlRender.ts"
TARGET="$REPO_ROOT/crates/tasks/src/artifact/html_render_bootstrap.html"
VARIABLES="$REPO_ROOT/crates/cli/src/commands/html_render_theme_variables.txt"

[ -f "$SOURCE" ] || { echo "no $SOURCE" >&2; exit 1; }

SCRIPT="$(mktemp --suffix=.ts)"
trap 'rm -f "$SCRIPT"' EXIT
# A head that already declares its charset and viewport gets only the style and
# the script, which is the part the Rust port does not build itself.
cat >"$SCRIPT" <<EOF
import { injectHtmlRenderBootstrap } from "$SOURCE";
const head = '<head><meta charset="utf-8"><meta name="viewport" content="width=device-width">';
const html = injectHtmlRenderBootstrap(head + "</head>");
process.stdout.write(html.slice("<head>".length, -(head.length - "<head>".length + "</head>".length)));
EOF
bun run "$SCRIPT" >"$TARGET"

# The guide's variable list, between its "live:" lead-in and the base stylesheet
# sentence, wrapped at 80 columns and indented for the help text.
cat >"$SCRIPT" <<EOF
import { HTML_RENDER_THEME_GUIDE as guide } from "$SOURCE";
const lead = "live: ";
const start = guide.indexOf(lead);
const end = guide.indexOf(" The base stylesheet");
if (start < 0 || end < start) throw new Error("HTML_RENDER_THEME_GUIDE changed shape");
const lines = [];
let line = " ";
for (const word of guide.slice(start + lead.length, end).split(" ")) {
  if (line.length + 1 + word.length > 78) {
    lines.push(line);
    line = " ";
  }
  line += " " + word;
}
lines.push(line);
console.log(lines.join("\n"));
EOF
bun run "$SCRIPT" >"$VARIABLES"
echo "Wrote $TARGET and $VARIABLES from $SOURCE"
