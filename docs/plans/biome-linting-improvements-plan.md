# Biome Linting Improvements Plan

**Status**: Active
**Last Updated**: 2025-10-13 (09:45)
**Next Rule**: `noGlobalIsNan` (Phase 1 telemetry)

## 🎯 Next Action

**Run telemetry baseline:**
```bash
bunx @biomejs/biome@latest lint --only lint/suspicious/noGlobalIsNan --diagnostic-level=warn --max-diagnostics=none
```

All reactivation steps for `noEmptyBlockStatements` are complete (warn + error). Begin the procedure again at Step 1 for the next rule.

## Rule Status Overview

### ✅ Completed Rules (error level enforced)

| Rule | Completed | Notes |
|------|-----------|-------|
| `noExcessiveCognitiveComplexity` | 2025-10-11 | 20+ files refactored (complexity ≤20) |
| `useSimplifiedLogicExpression` | 2025-10-07 | Auto-fixed 50 issues |
| `useBlockStatements` | 2025-10-08 | Enforces braces on control flow |
| `useConsistentArrayType` | 2025-10-08 | Enforces `T[]` syntax |
| `useExplicitLengthCheck` | 2025-10-06 | Requires `.length > 0` |
| `useTemplate` | 2025-10-09 | Template literals enforced |
| `useCollapsedElseIf` | 2025-10-10 | Prevents nested if pyramids |
| `useCollapsedIf` | 2025-10-10 | Simplifies redundant branches |
| `useFilenamingConvention` | 2025-10-10 | Kebab-case enforced |
| `useForOf` | 2025-10-06 | Prefers `for...of` loops |
| `noForEach` | 2025-10-12 | Manual refactors replaced `.forEach` usage |
| `useOptionalChain` | 2025-10-12 | Auto-fix + manual optional chaining cleanups |
| `noEmptyBlockStatements` | 2025-10-13 | Replaced noop blocks and documented intentional catches |

### 🔄 Rules Needing Work

| Rule | Status | Auto-Fix? | Priority | Notes |
|------|--------|-----------|----------|-------|
| `noGlobalIsNan` | off | ✅ | Medium | Suspicious: use `Number.isNaN` |
| `noDelete` | off | ❌ | Low | Performance: type updates required |
| `noExplicitAny` | warn | ❌ | Ongoing | Suspicious: gradual typing effort |
| `useAwait` | warn | ❌ | Low | Suspicious: remove redundant async |

## Telemetry Log

Run these commands to capture diagnostic counts before enabling each rule:

| Rule | Command | Diagnostics | Last Scan |
|------|---------|-------------|-----------|
| `noExcessiveCognitiveComplexity` | `bunx @biomejs/biome@latest lint --only lint/complexity/noExcessiveCognitiveComplexity --diagnostic-level=warn --max-diagnostics=none` | ✅ 0 | 2025-10-11 |
| `noForEach` | `bunx @biomejs/biome@latest lint --only lint/complexity/noForEach --diagnostic-level=warn --max-diagnostics=none` | ✅ 0 (commit 1aa5294) | 2025-10-12 |
| `useOptionalChain` | `bunx @biomejs/biome@latest lint --only lint/complexity/useOptionalChain --diagnostic-level=warn --max-diagnostics=none` | ✅ 0 (commit f20628f) | 2025-10-12 |
| `noEmptyBlockStatements` | `bunx @biomejs/biome@latest lint --only lint/suspicious/noEmptyBlockStatements --diagnostic-level=warn --max-diagnostics=none` | ⚠️ 44 diagnostics across 14 files (commit ccf7e38) | 2025-10-13 |
| `noGlobalIsNan` | `bunx @biomejs/biome@latest lint --only lint/suspicious/noGlobalIsNan --diagnostic-level=warn --max-diagnostics=none` | _TBD_ | _TBD_ |
| `noDelete` | `bunx @biomejs/biome@latest lint --only lint/performance/noDelete --diagnostic-level=warn --max-diagnostics=none` | _TBD_ | _TBD_ |

**After running a telemetry scan**, update the table with:
```
X diagnostics across Y files (commit abc1234)
```

## Rule Reactivation Procedure

Follow this workflow for **every** disabled rule. **Do not mark the task complete until you have executed _every_ step below, in order, during the current activation effort.** Skipping or deferring steps to a later task is not allowed.

1. **Run telemetry**: Capture baseline diagnostic count (see table above)
2. **Create branch**: `chore/biome-enable-<rule-name>`
3. **Enable as warn**: Change rule from `off` → `warn` in `biome.json`
4. **Run check**: `bun run biome:check`
5. **Fix violations**: Use `bun run biome:fix` for auto-fixable rules, manual fixes otherwise
6. **Verify clean**: Re-run `bun run biome:check` until zero diagnostics
7. **Commit warn**: `chore(biome): enable <rule> as warn`
8. **Promote to error**: Change `warn` → `error` in `biome.json`
9. **Run check again**: `bun run biome:check`
10. **Commit error**: `chore(biome): promote <rule> to error`
11. **Update this plan**: Mark rule as ✅ in tables above
12. **Open PR**: Get review before starting next rule

## Auto-Fix Reference

```bash
# Inspect diagnostics
bunx biome check --diagnostic-level=error --max-diagnostics=none

# Auto-fix (when rule supports it)
bunx biome check --write --diagnostic-level=error --max-diagnostics=none

# Or use project script
bun run biome:fix

# Always review before committing
git diff
```

### Which Rules Auto-Fix?

- ✅ Auto-fix: `useOptionalChain`, `noGlobalIsNan`, `useSimplifiedLogicExpression`, `useBlockStatements`, `useConsistentArrayType`, `useExplicitLengthCheck`, `useTemplate`
- ❌ Manual: `noExcessiveCognitiveComplexity`, `noForEach`, `noEmptyBlockStatements`, `noDelete`, `noExplicitAny`, `useAwait`

## Framework Overrides (Reference)

Rules are disabled in specific contexts:

- **Generated code** (`routeTree.gen.ts`): All linting disabled
- **Test files** (`*.test.ts`, `__tests__/**`): Relaxed rules for test readability
- **Storybook** (`*.stories.tsx`): Demos don't need production strictness
- **CSS** (`globals.css`): Tailwind v4 at-rules allowed

See `biome.json` for full override configuration.

## References

- [Biome Docs](https://biomejs.dev/linter/rules/)
- [Project biome.json](/biome.json)
