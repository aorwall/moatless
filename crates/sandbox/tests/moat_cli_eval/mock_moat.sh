#!/bin/sh
# Safe, scenario-driven moat shim for live agent evaluations.

set -u

jq -cn --args '$ARGS.positional | {argv: .}' -- "$@" >> "$MOAT_CLI_EVAL_LOG"

command_key=""
command_parts=0
skip_next=false
has_json=false
explicit_help=false

for arg in "$@"; do
    if [ "$skip_next" = true ]; then
        skip_next=false
        continue
    fi
    case "$arg" in
        -h|--help)
            explicit_help=true
            continue
            ;;
        --timeout)
            skip_next=true
            continue
            ;;
        --timeout=*|--verbose)
            continue
            ;;
        --json)
            has_json=true
            continue
            ;;
        -*)
            continue
            ;;
    esac
    if [ "$command_parts" -lt 2 ]; then
        if [ -n "$command_key" ]; then
            command_key="$command_key $arg"
        else
            command_key="$arg"
        fi
        command_parts=$((command_parts + 1))
    fi
done

if [ "$explicit_help" = true ] || [ "${command_key%% *}" = help ] || [ "${command_key#* }" = help ]; then
    "$MOAT_CLI_REAL_BIN" "$@"
    exit $?
fi

response=$(jq -ce --arg key "$command_key" '.mockResponses[$key] // empty' \
    "$MOAT_CLI_EVAL_SCENARIO_FILE")
if [ -z "$response" ]; then
    printf 'mock moat: no safe response configured for "%s"\n' "${command_key:-<empty>}" >&2
    exit 64
fi

if [ "$has_json" = true ]; then
    printf '%s\n' "$response" | jq -c '.json'
else
    printf '%s\n' "$response" | jq -r '.text // ""'
fi
exit_code=$(printf '%s\n' "$response" | jq -r '.exitCode // 0')
exit "$exit_code"
