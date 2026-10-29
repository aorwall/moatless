#!/bin/sh
# Git credential helper backed by the in-pod credential broker.
#
# Asks the broker on every call. It answers only the backend's own host, which
# every GitHub and Forgejo remote reaches through the git proxy, with the pod's
# task token; no GitHub token is ever held in the pod.
#
# Also the `GH_PATH` the sandbox binary exports. `gh` runs git with
# `-c credential.<host>.helper=!"$GH_PATH" auth git-credential`, which drops
# every other helper and would answer with `gh`'s placeholder token; through this
# script the call reaches the broker, and any other `gh` call passes through.

set -eu

SOCKET=/run/moatless/credentials.sock

if [ "${1:-}" = auth ] && [ "${2:-}" = git-credential ]; then
  shift 2
fi

case "${1:-}" in
  get)
    INPUT=$(cat)
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
