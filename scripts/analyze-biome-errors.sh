#!/bin/bash

echo "=== Running Biome React Hooks Analysis ==="
echo

# Run biome check and capture output
BIOME_OUTPUT=$(pnpm exec biome check apps/frontend/src --max-diagnostics=1000 2>&1)

# Count different types of errors
EXHAUSTIVE_DEPS=$(echo "$BIOME_OUTPUT" | grep -c "useExhaustiveDependencies")
HOOK_TOP_LEVEL=$(echo "$BIOME_OUTPUT" | grep -c "useHookAtTopLevel")
OTHER_ERRORS=$(echo "$BIOME_OUTPUT" | grep -c "lint/")

echo "React Hooks Errors Found:"
echo "- useExhaustiveDependencies: $EXHAUSTIVE_DEPS"
echo "- useHookAtTopLevel: $HOOK_TOP_LEVEL"
echo "- Other linting errors: $((OTHER_ERRORS - EXHAUSTIVE_DEPS - HOOK_TOP_LEVEL))"
echo

echo "Sample useExhaustiveDependencies errors:"
echo "$BIOME_OUTPUT" | grep -A5 "useExhaustiveDependencies" | head -30
echo

echo "Sample useHookAtTopLevel errors:"
echo "$BIOME_OUTPUT" | grep -A5 "useHookAtTopLevel" | head -30