#!/bin/sh
# Post-edit validation script that runs biome formatting and TypeScript type checking
# Returns exit code 2 on any errors

set -e

echo "🔍 Running post-edit validation..." >&2
echo "This checks for issues that may have been introduced by recent code changes." >&2
echo "" >&2
echo "⚠️  If you are still working on code changes, some errors below may be temporary." >&2
echo "   Continue with your planned fixes before addressing these errors." >&2
echo "" >&2

# Track if any errors occurred
HAS_ERRORS=0

# Run biome check with formatting
echo "📝 Running Biome formatter and linter..." >&2
if bun run biome check --write --diagnostic-level=error "$@"; then
  echo "✓ Biome check passed" >&2
else
  echo "" >&2
  echo "❌ Biome found errors in the code" >&2
  echo "These issues may be due to recent changes made by the agent." >&2
  echo "Please review and fix the errors above." >&2
  HAS_ERRORS=1
fi

echo "" >&2

# Run TypeScript type checking
echo "🔍 Running TypeScript type check..." >&2
# Run type check and capture output, filtering out transitive closure warnings
TYPE_CHECK_OUTPUT=$(bun run type:check 2>&1)
TYPE_CHECK_EXIT_CODE=$?

# Filter out the warnings and display the rest
echo "$TYPE_CHECK_OUTPUT" | grep -v "WARNING Unable to calculate transitive closures" | grep -v "No lockfile entry found for '@emnapi" >&2

# Check the actual exit code of the type check command
if [ $TYPE_CHECK_EXIT_CODE -eq 0 ]; then
  echo "✓ TypeScript type check passed" >&2
else
  echo "" >&2
  echo "❌ TypeScript found type errors" >&2
  echo "These type errors may be due to recent changes made by the agent." >&2
  echo "Please review and fix the type errors above." >&2
  HAS_ERRORS=1
fi

echo "" >&2

# Final status
if [ $HAS_ERRORS -eq 0 ]; then
  echo "✅ All validation checks passed!" >&2
  exit 0
else
  echo "⚠️  Validation failed!" >&2
  echo "The issues found above need to be addressed before proceeding." >&2
  echo "These errors may have been introduced by recent code modifications." >&2
  exit 2
fi