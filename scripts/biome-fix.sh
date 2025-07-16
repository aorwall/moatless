#!/bin/sh
# Wrapper script to run biome check and return exit code 2 on errors

biome check --write --diagnostic-level=error "$@"
EXIT_CODE=$?

if [ $EXIT_CODE -ne 0 ]; then
  exit 2
else
  exit 0
fi