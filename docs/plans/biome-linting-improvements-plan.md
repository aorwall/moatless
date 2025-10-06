# Biome Linting Improvements Plan

**Status**: Active
**Created**: 2025-10-06
**Total Issues**: 511 errors across 521 files

## Overview

This plan documents the Biome linting issues that were temporarily disabled or downgraded to warnings to allow the codebase to pass validation. These issues should be systematically fixed to improve code quality and maintainability.

## Summary Statistics

- **Backend Issues**: 304 errors
- **Frontend Issues**: 207 errors
- **Total Files Affected**: 521 files

### Top Issue Categories

1. **useBlockStatements** (style): 127 issues
2. **noExplicitAny** (suspicious): 107 issues
3. **noExcessiveCognitiveComplexity** (complexity): 77 issues
4. **useAwait** (suspicious): 55 issues
5. **noDelete** (performance): 36 issues

## Disabled/Downgraded Rules

### Style Rules (Off)

| Rule | Count | Backend | Frontend | Priority | Notes |
|------|-------|---------|----------|----------|-------|
| `useBlockStatements` | 127 | 41 | 86 | Medium | Forces braces for single-line if/for/while statements |
| `useConsistentArrayType` | 20 | 14 | 6 | Low | Enforces consistent array type syntax (T[] vs Array<T>) |
| `useExplicitLengthCheck` | 13 | 10 | 3 | Low | Prefers `.length > 0` over `.length` |
| `useTemplate` | 5 | 3 | 2 | Low | Prefers template literals over string concatenation |
| `useCollapsedElseIf` | 4 | 1 | 3 | Low | Combines nested if statements |
| `useCollapsedIf` | 3 | 3 | 0 | Low | Combines nested if statements |
| `useFilenamingConvention` | 2 | 2 | 0 | Low | Enforces kebab-case file naming |
| `useForOf` | 1 | 1 | 0 | Low | Prefers for-of over traditional for loops |

### Complexity Rules

| Rule | Count | Backend | Frontend | Priority | Notes |
|------|-------|---------|----------|----------|-------|
| `noExcessiveCognitiveComplexity` | 77 | 54 | 23 | Medium | Disabled - Max complexity: 15 |
| `noForEach` | 15 | 14 | 1 | Low | Disabled - Prefers for-of over forEach |
| `useOptionalChain` | 9 | 9 | 0 | Low | Disabled - Prefers optional chaining (?.) |

### Suspicious Rules

| Rule | Count | Backend | Frontend | Priority | Notes |
|------|-------|---------|----------|----------|-------|
| `noExplicitAny` | 107 | 82 | 25 | High | Downgraded to **warn** - Should fix gradually |
| `useAwait` | 55 | 39 | 16 | High | Downgraded to **warn** - Remove unnecessary async |
| `noEmptyBlockStatements` | 11 | 8 | 3 | Low | Disabled - Empty catch/try blocks |
| `noGlobalIsNan` | 2 | 2 | 0 | Low | Disabled - Prefers Number.isNaN |

### Performance Rules

| Rule | Count | Backend | Frontend | Priority | Notes |
|------|-------|---------|----------|----------|-------|
| `noDelete` | 36 | 36 | 0 | Medium | Disabled - Prefers setting to undefined |

## Recommended Fix Priority

### Phase 1: High Priority (Critical for Type Safety)
**Target**: 2-3 weeks

1. ✅ **noExplicitAny (107 issues)** - Currently **warn**
   - Replace with proper types or `unknown`
   - Use type guards where needed
   - Document legitimate uses with `biome-ignore` comments

2. ✅ **useAwait (55 issues)** - Currently **warn**
   - Remove unnecessary `async` keywords
   - Use `Promise.resolve()` / `Promise.reject()` for sync functions returning promises

### Phase 2: Medium Priority (Code Quality)
**Target**: 1-2 weeks

3. ✅ **noExcessiveCognitiveComplexity (77 issues)** - Currently **warn**
   - Refactor complex functions (complexity > 15)
   - Extract helper functions
   - Simplify conditional logic

4. ✅ **noDelete (36 issues)**
   - Replace `delete obj.prop` with `obj.prop = undefined`
   - Update TypeScript types to allow `undefined`

5. ✅ **useBlockStatements (127 issues)**
   - Add braces to single-line if/for/while statements
   - Can be auto-fixed with `biome check --write`

### Phase 3: Low Priority (Style Consistency)
**Target**: 1 week

6. ✅ **useConsistentArrayType (20 issues)**
   - Standardize on `T[]` or `Array<T>` syntax
   - Can be auto-fixed

7. ✅ **Other Style Rules (~30 combined issues)**
   - useExplicitLengthCheck
   - useTemplate
   - useCollapsedElseIf
   - useCollapsedIf
   - useForOf
   - Can mostly be auto-fixed

## Auto-Fix Strategy

Many issues can be auto-fixed:

```bash
# Check fixable issues
bun run biome:check

# Auto-fix all safe issues
bun run biome:fix

# Review changes before committing
git diff
```

### Rules with Auto-Fix

- ✅ `useBlockStatements` - Safe to auto-fix
- ✅ `useConsistentArrayType` - Safe to auto-fix
- ✅ `useExplicitLengthCheck` - Safe to auto-fix
- ✅ `useTemplate` - Mostly safe to auto-fix
- ✅ `useOptionalChain` - Safe to auto-fix
- ✅ `noUselessConstructor` - Safe to auto-fix
- ❌ `noExplicitAny` - Requires manual review
- ❌ `noExcessiveCognitiveComplexity` - Requires refactoring
- ❌ `noDelete` - Requires type updates

## Configuration Improvements

Consider adopting these from the blog post:

1. **Add experimentalScannerIgnores**:
   ```json
   "experimentalScannerIgnores": [
     "**/node_modules/**",
     "**/.next/**",
     "**/dist/**",
     "**/coverage/**"
   ]
   ```

2. **More specific file includes** (if needed for performance)

3. **Consider test overrides** (already implemented ✅)

## Tracking Progress

### Current State (as of 2025-10-06)

```typescript
// Rules set to "warn" (should eventually be "error")
"noExplicitAny": "warn",              // 107 issues
"useAwait": "warn",                    // 55 issues

// Rules set to "off" (should eventually be "error")
"noExcessiveCognitiveComplexity": "off", // 77 issues - moved to "off" from "warn"
"useBlockStatements": "off",           // 127 issues
"useCollapsedElseIf": "off",          // 4 issues
"useCollapsedIf": "off",              // 3 issues
"useConsistentArrayType": "off",       // 20 issues
"useExplicitLengthCheck": "off",       // 13 issues
"useFilenamingConvention": "off",      // 2 issues
"useForOf": "off",                     // 1 issue
"useTemplate": "off",                  // 5 issues
"noForEach": "off",                    // 15 issues
"useOptionalChain": "off",             // 9 issues
"noEmptyBlockStatements": "off",       // 11 issues
"noGlobalIsNan": "off",                // 2 issues
"noDelete": "off"                      // 36 issues

// Nursery rules
"noUnknownAtRule": "off",              // Disabled only for CSS files (see overrides)
"useIterableCallbackReturn": "off"     // Disabled globally - allows forEach with side effects

// Configuration options
"noUnusedVariables": {                 // Enabled ignoreRestSiblings for destructuring patterns
  "level": "error",
  "options": {
    "ignoreRestSiblings": true
  }
}
```

### Progress Checklist

- [ ] Phase 1: Fix high priority issues (noExplicitAny, useAwait)
- [ ] Phase 2: Fix medium priority issues (complexity, noDelete, useBlockStatements)
- [ ] Phase 3: Fix low priority style issues
- [ ] Re-enable all rules as "error"
- [ ] Update pre-commit hooks to enforce all rules
- [ ] Document any permanent exceptions with biome-ignore comments

## Framework-Specific Overrides

We've configured targeted overrides to disable linting rules where they conflict with framework patterns:

### 1. **Generated Code** (Complete Disable)
```json
{
  "includes": ["apps/frontend/src/routeTree.gen.ts"],
  "linter": { "enabled": false }
}
```
- **Why**: Auto-generated by TanStack Router
- **Rules**: All disabled

### 2. **Storybook Files** (Selective Disable)
```json
{
  "includes": ["**/*.stories.tsx", "**/*.stories.ts"],
  "rules": {
    "noArrayIndexKey": "off",         // Mock data often uses array indices
    "noExplicitAny": "off",            // Story args can be loosely typed
    "noExcessiveCognitiveComplexity": "off"  // Stories can be complex for demos
  }
}
```
- **Why**: Storybook stories are demos/examples, not production code
- **Rules**: Relaxed type safety and complexity

### 3. **CSS Files** (Tailwind v4 Support)
```json
{
  "includes": ["**/globals.css", "**/*.css"],
  "rules": {
    "noUnknownAtRule": "off"  // Allows @apply, @theme, @plugin, @custom-variant
  }
}
```
- **Why**: Tailwind v4 uses custom at-rules (`@apply`, `@theme`, `@plugin`, `@custom-variant`)
- **Rules**: Only disabled `noUnknownAtRule` (nursery rule)

### 4. **Test Files** (Comprehensive Disable)
```json
{
  "includes": ["**/*.test.ts", "**/*.test.tsx", "**/__tests__/**/*", "**/test-utils/**/*"],
  "rules": {
    "suspicious": { "noExplicitAny": "off", "noConsole": "off", ... },
    "style": { "noNonNullAssertion": "off", "useBlockStatements": "off" },
    "complexity": { "noExcessiveCognitiveComplexity": "off", ... },
    "performance": { "noDelete": "off" }
  }
}
```
- **Why**: Tests prioritize readability and ease of writing over production strictness
- **Rules**: Many relaxed (documented in biome.json overrides section)

## Notes

- ✅ **Framework conflicts resolved** via file-specific overrides (not global disables)
- ✅ **Test files** have looser rules via overrides
- ✅ **CSS/Tailwind** rules only disabled for CSS files (not globally)
- ✅ **Storybook** gets special treatment for demo code patterns
- Consider gradually fixing files as they're touched rather than a big-bang approach
- Track progress by moving rules from "off" → "warn" → "error" over time

## References

- [Biome Documentation](https://biomejs.dev/linter/rules/)
- [Current biome.json](/biome.json)
- [Pre-commit Hook](/.husky/pre-commit)
