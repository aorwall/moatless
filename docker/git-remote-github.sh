#!/bin/sh
# Git remote helper for `git+https://github.com/...`, installed as
# `git-remote-git+https`. The sandbox rewrites every github.com URL to that
# scheme with `url.git+https://github.com/.insteadOf` (`init::git`).
#
# The transfer goes to the backend git proxy, which holds the GitHub
# credential; git's credential helper answers the proxy with the task token.
# The scheme is deliberate: `gh` reads a `git+https` remote as the github.com
# repository it names, while an `insteadOf` straight to the proxy would show
# `gh` a remote on no GitHub host, and `gh pr checkout` would refuse it.
#
# `moatless-github.<owner/name>.url` maps a registered repository to its
# `/git/{id}` proxy URL; any other repository goes to
# `moatless-github.proxy`/<owner>/<name>.

set -eu

URL=${2:-$1}
URL=${URL#git+}
case "$URL" in
  https://github.com/*) ;;
  *) exec git remote-https "$1" "$URL" ;;
esac

REPO=${URL#https://github.com/}
REPO=${REPO%/}
REPO=${REPO%.git}
KEY=$(printf '%s' "$REPO" | tr '[:upper:]' '[:lower:]')

TARGET=$(git config --get "moatless-github.$KEY.url" || true)
if [ -z "$TARGET" ]; then
  BASE=$(git config --get moatless-github.proxy || true)
  if [ -z "$BASE" ]; then
    echo "git-remote-git+https: no backend git proxy is configured; fetching $URL directly" >&2
    exec git remote-https "$1" "$URL"
  fi
  TARGET="$BASE/$REPO"
fi
exec git remote-https "$1" "$TARGET"
