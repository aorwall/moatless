#!/bin/sh
# Git credential helper backed by the in-pod credential broker.
#
# Pulls a fresh token from the broker on every call, eliminating the
# staleness window of the periodic hosts.yml refresher. Falls back to
# `gh auth git-credential` if the broker is unreachable so the agent
# still has a working credential path during a backend outage.

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
      exit 0
    fi
    printf '%s\n' "$INPUT" | exec gh auth git-credential get
    ;;
  store|erase|*)
    cat >/dev/null
    ;;
esac
