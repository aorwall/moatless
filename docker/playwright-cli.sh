#!/bin/sh
# Wrapper for `playwright cli`, the agent-facing Playwright surface (1.62.0+).
#
# `--browser` is valid on `open` alone; every other subcommand exits with
# "Unknown option: --browser". The CLI defaults to the `chrome` channel at
# /opt/google/chrome/chrome, which this image does not install, so `open` must
# carry the flag and nothing else may.

: "${PLAYWRIGHT_CLI_BIN:=/opt/playwright-packages/${PLAYWRIGHT_CLI_VERSION}/node_modules/.bin/playwright}"

if [ ! -x "$PLAYWRIGHT_CLI_BIN" ]; then
    echo "playwright-cli: no Playwright at $PLAYWRIGHT_CLI_BIN" >&2
    exit 127
fi

subcommand=
for arg in "$@"; do
    case "$arg" in
        -*) ;;
        *)
            subcommand=$arg
            break
            ;;
    esac
done

if [ "$subcommand" = open ]; then
    case " $* " in
        *" --browser"*) ;;
        *) set -- "$@" --browser=chromium ;;
    esac
fi

exec xvfb-run --auto-servernum "$PLAYWRIGHT_CLI_BIN" cli "$@"
