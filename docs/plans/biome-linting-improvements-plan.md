# Biome Linting Improvements Plan

**Status**: Active
**Last Updated**: 2025-10-11
**Current State**: ✅ All enabled rules passing

## Quick Status

```bash
bunx biome check --diagnostic-level=error --reporter summary
# Checked 507 files in 133ms. No fixes applied. ✓
```

## Next Task for Agents

**Pick the next unchecked rule from the table below, enable it, fix all violations, and commit.**

Follow this workflow:
1. Pick next ☐ rule from "Rule Status" table
2. Enable as `warn` in `biome.json`
3. Run `bunx biome check --diagnostic-level=warn`
4. Fix all violations (use `bunx biome check --write` for auto-fixable rules)
5. Commit with message: `chore(biome): enable [rule-name] as warn`
6. Promote to `error` in `biome.json`
7. Run `bunx biome check --diagnostic-level=error`
8. Fix any additional violations
9. Commit with message: `chore(biome): promote [rule-name] to error`
10. Update this doc: mark rule as ✅ in both Warn and Error columns

## Rule Status

### Style Rules ✅ Complete

| Rule | Warn | Error | Auto-fix |
|------|------|-------|----------|
| `useSimplifiedLogicExpression` | ✅ | ✅ | Yes |
| `useBlockStatements` | ✅ | ✅ | Yes |
| `useConsistentArrayType` | ✅ | ✅ | Yes |
| `useExplicitLengthCheck` | ✅ | ✅ | Yes |
| `useTemplate` | ✅ | ✅ | Yes |
| `useCollapsedElseIf` | ✅ | ✅ | Yes |
| `useCollapsedIf` | ✅ | ✅ | Yes |
| `useFilenamingConvention` | ✅ | ✅ | No |
| `useForOf` | ✅ | ✅ | Yes |

### Complexity Rules

| Rule | Warn | Error | Auto-fix | Notes |
|------|------|-------|----------|-------|
| `noExcessiveCognitiveComplexity` | ✅ | ✅ | No | 20+ files refactored (2025-10-11) |
| `noForEach` | ☐ | ☐ | Yes | **NEXT**: Enable this rule |
| `useOptionalChain` | ☐ | ☐ | Yes | After `noForEach` |

### Suspicious Rules

| Rule | Warn | Error | Auto-fix | Notes |
|------|------|-------|----------|-------|
| `noExplicitAny` | ⚠️ | ☐ | No | Currently at `warn` - needs gradual typing plan |
| `useAwait` | ⚠️ | ☐ | No | Currently at `warn` - audit redundant async |
| `noEmptyBlockStatements` | ☐ | ☐ | No | Document intentional empties first |
| `noGlobalIsNan` | ☐ | ☐ | Yes | Replace with `Number.isNaN` |

### Performance Rules

| Rule | Warn | Error | Auto-fix | Notes |
|------|------|-------|----------|-------|
| `noDelete` | ☐ | ☐ | No | Requires type updates for `undefined` |

## Telemetry Commands

Before enabling a rule, check its current impact:

```bash
# Example for noForEach
bunx biome lint --only lint/performance/noForEach \
  --diagnostic-level=warn --max-diagnostics=none

# Count violations
bunx biome lint --only lint/performance/noForEach \
  --diagnostic-level=warn --max-diagnostics=none \
  --reporter json | jq '.diagnostics | length'
```

## Framework Overrides

The following overrides are configured in `biome.json`:

- **Generated code** (`routeTree.gen.ts`): All linting disabled
- **Storybook** (`**/*.stories.tsx`): Relaxed complexity and type rules
- **Tests** (`**/*.test.ts`, `**/__tests__/**`): Relaxed rules for test clarity
- **Tailwind CSS** (`globals.css`): `noUnknownAtRules` disabled for Tailwind v4

## References

- [Biome Documentation](https://biomejs.dev/linter/rules/)
- [biome.json](/biome.json)
