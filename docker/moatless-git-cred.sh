#!/bin/sh
# Git credential helper backed by the in-pod credential broker.
#
# Pulls a fresh token from the broker on every call. Answers nothing when the
# broker is unreachable: `gh` holds only a placeholder token, so it has no
# credential to fall back to.
#
# Also the `GH_PATH` the sandbox binary exports. `gh` runs git with
# `-c credential.<host>.helper=!"$GH_PATH" auth git-credential`, which drops
# every other helper and would answer with that placeholder; through this
# script the call reaches the broker, and any other `gh` call passes through.

set -eu

SOCKET=/run/moatless/credentials.sock

if [ "${1:-}" = auth ] && [ "${2:-}" = git-credential ]; then
  shift 2
fi

case "${1:-}" in
  get)
    INPUT=$(cat)
    # The broker answers per host: the backend's own host is a proxied checkout,
    # which is given the pod's task token instead of a git provider token.
    HOST=$(printf '%s\n' "$INPUT" | sed -n 's/^host=//p' | head -n 1)
    START=$(date +%s)
    if RESULT=$(curl --silent --max-time 5 --fail --unix-socket "$SOCKET" --get --data-urlencode "host=$HOST" http://./git-credential 2>/dev/null); then
      printf '%s\n' "$RESULT"
    else
      # Git reports an empty answer only as `could not read Username`, so name
      # the cause: exit 28 is the timeout, 22 an error status from the broker.
      RC=$?
      echo "moatless-git-cred: no credential for $HOST (curl exit $RC after $(( $(date +%s) - START ))s)" >&2
    fi
    ;;
  store|erase)
    cat >/dev/null
    ;;
  *)
    exec gh "$@"
    ;;
esac
