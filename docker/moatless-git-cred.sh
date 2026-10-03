#!/bin/sh
# Git credential helper backed by the in-pod credential broker.
#
# Pulls a fresh token from the broker on every call. Answers nothing when the
# broker is unreachable: `gh` holds only a placeholder token, so it has no
# credential to fall back to.

set -eu

SOCKET=/run/moatless/credentials.sock

case "${1:-}" in
  get)
    INPUT=$(cat)
    # The broker answers per host: the backend's own host is a proxied checkout,
    # which is given the pod's task token instead of a git provider token.
    HOST=$(printf '%s\n' "$INPUT" | sed -n 's/^host=//p' | head -n 1)
    if RESULT=$(curl --silent --max-time 5 --fail --unix-socket "$SOCKET" --get --data-urlencode "host=$HOST" http://./git-credential 2>/dev/null); then
      printf '%s\n' "$RESULT"
    fi
    ;;
  store|erase|*)
    cat >/dev/null
    ;;
esac
