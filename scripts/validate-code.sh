#!/bin/sh
# Post-edit validation script that runs biome formatting and TypeScript type checking
# Returns exit code 2 on any errors

set -e

# Colors for output
RED='\033[0;31m'
YELLOW='\033[1;33m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

echo "${YELLOW}🔍 Running post-edit validation...${NC}"
echo "${YELLOW}This checks for issues that may have been introduced by recent code changes.${NC}"
echo ""

# Track if any errors occurred
HAS_ERRORS=0

# Run biome check with formatting
echo "${YELLOW}📝 Running Biome formatter and linter...${NC}"
if pnpm exec biome check --write --diagnostic-level=error "$@"; then
  echo "${GREEN}✓ Biome check passed${NC}"
else
  echo ""
  echo "${RED}❌ Biome found errors in the code${NC}"
  echo "${RED}These issues may be due to recent changes made by the agent.${NC}"
  echo "${RED}Please review and fix the errors above.${NC}"
  HAS_ERRORS=1
fi

echo ""

# Run TypeScript type checking
echo "${YELLOW}🔍 Running TypeScript type check...${NC}"
if pnpm type:check; then
  echo "${GREEN}✓ TypeScript type check passed${NC}"
else
  echo ""
  echo "${RED}❌ TypeScript found type errors${NC}"
  echo "${RED}These type errors may be due to recent changes made by the agent.${NC}"
  echo "${RED}Please review and fix the type errors above.${NC}"
  HAS_ERRORS=1
fi

echo ""

# Final status
if [ $HAS_ERRORS -eq 0 ]; then
  echo "${GREEN}✅ All validation checks passed!${NC}"
  exit 0
else
  echo "${RED}⚠️  Validation failed!${NC}"
  echo "${RED}The issues found above need to be addressed before proceeding.${NC}"
  echo "${RED}These errors may have been introduced by recent code modifications.${NC}"
  exit 2
fi