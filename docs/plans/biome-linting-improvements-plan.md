# Biome Linting Improvements Plan

**Status**: Active
**Last Updated**: 2025-10-13 (22:10)
**Next Rule**: `noExplicitAny` (Phase 1 remediation)

## 🎯 Next Action

**Run telemetry baseline:**
```bash
bunx @biomejs/biome@latest lint --only lint/suspicious/noExplicitAny --diagnostic-level=warn --max-diagnostics=none
```

All reactivation steps for `noDelete` are complete (warn + error). We have now executed Step&nbsp;1 for `noExplicitAny`; continue with the remediation actions below before promoting the rule.

### `noExplicitAny` remediation game plan

1. **Capture baseline diagnostics (✅ complete):**
   ```bash
   bunx @biomejs/biome@latest lint --only lint/suspicious/noExplicitAny --diagnostic-level=warn --max-diagnostics=none
   ```
   *Result:* 576 diagnostics across stories, tests, mocks, and a handful of runtime components.
2. **Bucket the violations:**
   - _Storybook stories_: swap `args: {} as any` for properly typed `Story['args']` objects or remove empty `args` blocks.
   - _Frontend runtime code_: replace `as any` casts with real types or `unknown` → specific-type casts (e.g. `useApiError`).
   - _Test utilities & mocks_: prefer explicit helper interfaces/`unknown` bridges instead of `any`.
   - _E2E helpers_: extend the Playwright `window` type with explicit SSE helpers.
3. **Fix per bucket (in warn mode):**
   - Batch through stories first—they account for ~70% of the diagnostics and can mostly drop the `args` keys. _(Deferred: continue focusing on runtime surfaces while stories remain ignored.)_
   - Replace remaining `as any` in tests/mocks with typed helpers; introduce targeted utilities when repetition appears.
   - ✅ Harden backend runtime surfaces: typed environment agent access, worker run options, Claude usage telemetry, and Netlify deployment updates to remove `any` casts.
   - ✅ Frontend runtime & test utilities: typed API error handling, event invalidation map, task header status usage, and SSE/WebSocket mocks without `any` escapes.
   - ✅ Task runtime surfaces: task card todo parsing + iteration preview typing, repository creation form field updates, and command availability checks without `any`.
   - ✅ Workspace messaging & setup lifecycle: message service dependency wiring, workspace path updates, and MCP setup guards without `any` casts.
4. **Promote to `error`:** once `bun run biome:check --write` reports zero diagnostics for `noExplicitAny`, flip the rule to `error` and rerun the check to confirm.
5. **Documentation:** capture the `bunx` command output in the telemetry table and list any helper utilities you introduced so future tasks know where to look.

> ⚠️ **Important:** `noExplicitAny` is currently disabled for tests, stories, and generated files via `biome.json` overrides. When you touch those paths, either remove the overrides or keep fixes aligned with the patterns above so we can safely re-enable the rule later.

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
| `noGlobalIsNan` | 2025-10-13 | Replaced `isNaN` checks with `Number.isNaN` in preview inspector |
| `noEmptyBlockStatements` | 2025-10-13 | Replaced noop blocks and documented intentional catches |
| `noDelete` | 2025-10-13 | Swapped delete usage for `Reflect.deleteProperty`/safe resets across tests & services |

### 🔄 Rules Needing Work

| Rule | Status | Auto-Fix? | Priority | Notes |
|------|--------|-----------|----------|-------|
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
| `noGlobalIsNan` | `bunx @biomejs/biome@latest lint --only lint/suspicious/noGlobalIsNan --diagnostic-level=warn --max-diagnostics=none` | ⚠️ 2 diagnostics across 1 file (commit bbbeabc) | 2025-10-13 |
| `noDelete` | `bunx @biomejs/biome@latest lint --only lint/performance/noDelete --diagnostic-level=warn --max-diagnostics=none` | ⚠️ 61 diagnostics across 18 files (commit 56178f0) | 2025-10-13 |
| `noExplicitAny` | `bunx @biomejs/biome@latest lint --only lint/suspicious/noExplicitAny --diagnostic-level=warn --max-diagnostics=none` | ⚠️ 576 diagnostics across 187 files (baseline captured locally) | 2025-10-13 |

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
