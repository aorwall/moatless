#!/usr/bin/env bash
# Fail when a repo in docker/playwright-consumers.json resolves a playwright-core
# build that docker/Dockerfile.sandbox-base does not bake, or when
# docker/Dockerfile.e2e's base tag disagrees with this repo's own lockfile.
#
# The image installs one Chromium per version in PLAYWRIGHT_VERSION,
# PLAYWRIGHT_CLI_VERSION and PLAYWRIGHT_COMPAT_VERSIONS. A consumer that moves
# its pin off that set gets "Executable doesn't exist at
# /opt/playwright-browsers/chromium-<build>" and downloads at test time, which
# sandbox egress rules block. Nothing else compares the two sides.
#
# Reading a private consumer needs GH_TOKEN with repo:read. Without one the
# remote consumers are skipped with a warning rather than failing, so a fork PR
# still runs the local half.

set -euo pipefail

DOCKERFILE=docker/Dockerfile.sandbox-base
CONSUMERS=docker/playwright-consumers.json

arg_value() { sed -n "s/^ARG $1=\"\\?\\([^\"]*\\)\"\\?\$/\\1/p" "$DOCKERFILE"; }

baked=$(
	{
		arg_value PLAYWRIGHT_VERSION
		arg_value PLAYWRIGHT_CLI_VERSION
		arg_value PLAYWRIGHT_COMPAT_VERSIONS | tr ' ' '\n'
	} | grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' | sort -u
)

if [ -z "$baked" ]; then
	echo "::error::No Playwright versions found in $DOCKERFILE — expected ARG PLAYWRIGHT_VERSION, ARG PLAYWRIGHT_CLI_VERSION and ARG PLAYWRIGHT_COMPAT_VERSIONS"
	exit 1
fi

echo "$DOCKERFILE bakes: $(echo "$baked" | tr '\n' ' ')"

failed=0
skipped=0
claimed=""
while read -r repo lockfile is_local; do
	if [ "$is_local" = "true" ]; then
		if [ ! -f "$lockfile" ]; then
			echo "::error::$repo declares $lockfile, which is not in this checkout"
			failed=1
			continue
		fi
		content=$(cat "$lockfile")
	elif [ -z "${GH_TOKEN:-}" ]; then
		echo "::warning::Skipped $repo — GH_TOKEN is unset, so its $lockfile cannot be read"
		skipped=1
		continue
	# The raw media type, deliberately: the JSON one populates `content` only at
	# or under 1 MiB and answers 200 with an empty string above it, which would
	# read here as a consumer with no Playwright pin and pass the check silently.
	elif ! content=$(gh api -H "Accept: application/vnd.github.raw" "repos/$repo/contents/$lockfile" 2>/dev/null); then
		echo "::error::Could not read $lockfile from $repo. Fix the path in $CONSUMERS, drop the entry, or widen the token's access"
		failed=1
		skipped=1
		continue
	fi

	# The hoisted resolution of each Playwright package, which is what an
	# importer in the repo gets. A copy nested under an unrelated dependency
	# (`"likec4/playwright/playwright-core"`) is deliberately not read: nothing
	# in the repo imports it. The gap that leaves is a workspace member whose
	# own pin lost the hoist to a different one.
	wanted=$(printf '%s' "$content" |
		{ grep -oE '"(playwright|playwright-core|@playwright/test)": \["[^"]*@[0-9]+\.[0-9]+\.[0-9]+"' || true; } |
		sed -E 's/.*@([0-9]+\.[0-9]+\.[0-9]+)"$/\1/' | sort -u)
	if [ -z "$wanted" ]; then
		echo "::warning::$repo resolves no playwright-core in $lockfile — drop it from $CONSUMERS if it no longer runs browser tests"
		continue
	fi

	for version in $wanted; do
		claimed="$claimed $version "
		if echo "$baked" | grep -qx "$version"; then
			echo "ok    $repo needs $version"
		else
			echo "::error::$repo resolves playwright-core $version, which $DOCKERFILE does not bake. Add it to ARG PLAYWRIGHT_COMPAT_VERSIONS, or move the consumer onto a baked version."
			failed=1
		fi
	done
done < <(jq -r '.consumers[] | [.repo, .lockfile, (.local // false)] | @tsv' "$CONSUMERS")

# A compat entry exists for a consumer and nothing else, so one no consumer
# resolves is a browser build the image carries for nobody. Say so: the loop
# above prints a line per consumer and never per unclaimed version, which is how
# the list this replaced came to name a repo with no Playwright dependency.
# Skipped consumers make the set incomplete, so this only runs on a full pass.
if [ "$skipped" = "0" ]; then
	for version in $(arg_value PLAYWRIGHT_COMPAT_VERSIONS); do
		case "$claimed" in
		*" $version "*) ;;
		*) echo "::warning::No consumer resolves playwright-core $version — drop it from ARG PLAYWRIGHT_COMPAT_VERSIONS in $DOCKERFILE" ;;
		esac
	done
fi

# The e2e runner needs equality, not membership: `FROM playwright:v${VERSION}`
# is an exact tag, so it carries one browser set and a mismatched runner
# launches browsers the suite was not built against. `bun.lock` rather than
# `package.json`, deliberately — the caret floor there never moves under a
# `bun update`, so it agrees with a stale pin.
E2E_DOCKERFILE=docker/Dockerfile.e2e
e2e_pin=$(sed -n 's/^ARG PLAYWRIGHT_VERSION=\([0-9][^"]*\)$/\1/p' "$E2E_DOCKERFILE")
local_pin=$(
	{ grep -oE '"@playwright/test": \["@playwright/test@[0-9]+\.[0-9]+\.[0-9]+"' bun.lock || true; } |
		sed -E 's/.*@([0-9]+\.[0-9]+\.[0-9]+)"$/\1/' | sort -u
)
if [ -z "$e2e_pin" ] || [ -z "$local_pin" ]; then
	echo "::error::Could not read a Playwright version — expected one 'ARG PLAYWRIGHT_VERSION=X.Y.Z' in $E2E_DOCKERFILE and one hoisted '@playwright/test' entry in bun.lock"
	failed=1
elif [ "$e2e_pin" != "$local_pin" ]; then
	echo "::error::bun.lock resolves @playwright/test $local_pin but $E2E_DOCKERFILE bakes $e2e_pin. Move them together."
	failed=1
else
	echo "ok    $E2E_DOCKERFILE matches bun.lock at $e2e_pin"
fi

exit "$failed"
