#!/bin/bash

# Run type check and filter out WASI-related warnings
bun run --silent turbo run type:check 2>&1 | grep -v "WARNING.*Unable to calculate transitive closures" | grep -v "No lockfile entry found for '@emnapi"
exit_code=${PIPESTATUS[0]}
exit $exit_code