#!/bin/sh
# Wrapper for `playwright cli`, the agent-facing Playwright surface (1.62.0+).
#
# `--browser` is valid on `open` alone; every other subcommand exits with
# "Unknown option: --browser". The CLI defaults to the `chrome` channel at
# /opt/google/chrome/chrome, which this image does not install, so `open` must
# carry the flag and nothing else may.

export PLAYWRIGHT_BROWSERS_PATH=/opt/playwright-cli-browsers

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

exec xvfb-run --auto-servernum npx "playwright@${PLAYWRIGHT_CLI_VERSION}" cli "$@"
