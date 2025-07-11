#!/bin/bash

echo "=== React Performance Audit ==="
echo

# Find effects that might be setting derived state
echo "1. Potential derived state in effects:"
grep -r "useEffect" apps/frontend/src --include="*.tsx" --include="*.ts" -A 5 | grep -B 5 "set[A-Z]" | grep -v "test" | head -20

echo
echo "2. Components with many hooks (complexity):"
grep -r "use[A-Z]" apps/frontend/src --include="*.tsx" --include="*.ts" | grep -v "test" | cut -d: -f1 | sort | uniq -c | sort -nr | head -10

echo
echo "3. Inline objects/functions in dependencies:"
grep -r "useEffect\|useMemo\|useCallback" apps/frontend/src --include="*.tsx" --include="*.ts" -A 2 | grep -E "\[[^\]]*{|\[[^\]]*=>" | head -10

echo
echo "4. Context providers with inline values:"
grep -r "\.Provider" apps/frontend/src --include="*.tsx" --include="*.ts" | grep "value={" | grep -v "useMemo" | head -10

echo
echo "5. Effects with many dependencies (>3):"
grep -r "useEffect" apps/frontend/src --include="*.tsx" --include="*.ts" -A 1 | grep -E "\[[^\]]{30,}" | head -10