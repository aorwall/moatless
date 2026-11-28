#!/usr/bin/env bash
# Fail when THIRD_PARTY_NOTICES.md names an agent CLI version that
# docker/cli-versions.env does not pin. The notice states what the public
# sandbox image ships, so a bump that misses it publishes a wrong notice.

set -euo pipefail

NOTICES=THIRD_PARTY_NOTICES.md
PINS=docker/cli-versions.env

failed=0
while read -r key package; do
	pinned=$(sed -n "s/^${key}=//p" "$PINS")
	noted=$(sed -n "s|^\`${package}\` \([^ ]*\)\$|\1|p" "$NOTICES")
	if [ -z "$pinned" ] || [ -z "$noted" ]; then
		echo "::error::Expected ${key}= in $PINS and a line \`${package}\` <version> in $NOTICES"
		failed=1
	elif [ "$pinned" != "$noted" ]; then
		echo "::error::$NOTICES names ${package} ${noted}, but $PINS pins ${pinned}. Update the version in $NOTICES."
		failed=1
	else
		echo "${package} ${pinned}: matches"
	fi
done <<'EOF'
CLAUDE_CLI_VERSION @anthropic-ai/claude-code
CODEX_CLI_VERSION @openai/codex
PI_CLI_VERSION @earendil-works/pi-coding-agent
EOF

exit "$failed"
