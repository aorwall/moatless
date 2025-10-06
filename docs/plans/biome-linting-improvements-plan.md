# Biome Linting Improvements Plan

**Status**: Active
**Created**: 2025-10-06
**Last Updated**: 2025-10-09
**Total Issues**: 0 active errors (historical peak: 50 across 26 files, 520 files scanned)

## Overview

This plan documents the Biome linting issues that were temporarily disabled or downgraded to warnings to allow the codebase to pass validation. An earlier snapshot from `biome check --diagnostic-level=error --reporter summary` (taken immediately after syncing with the latest `main`) showed that `useSimplifiedLogicExpression` was the lone failing rule with 50 diagnostics. After the 2025-10-07 cleanup pass, the same command runs clean, but the remaining disabled or downgraded rules still need to be reintroduced once the new baseline holds.

To keep the rollout predictable, the team will reactivate rules **one at a time**, always running `bun run biome:check` after each configuration change, fixing every diagnostic, and committing before moving on to the next rule. The detailed procedure appears below and must be followed for every rule with zero exceptions. **Whenever you pick up this work, immediately grab the next unchecked rule from the Phase 1 checklist, flip it from `off` to `warn`, and run `bun run biome:check` before making any other edits so we never skip the enable→lint sequence.**

## Immediate Work in Flight

- ✅ **Baseline verified**: `git fetch origin main && git rebase origin/main` followed by `bun run biome:check` on 2025-10-07 confirmed the 50 failing diagnostics are limited to `useSimplifiedLogicExpression`.
- 🚧 **Autofix rehearsal**: A draft branch `chore/biome-fix-use-simplified-logic` is running `bun run biome:fix --files "apps/**" "packages/**"` to validate auto-fixes. Screenshots and diffs are being reviewed with the affected service owners before merging.
- ✅ **String template audit**: Raised `useTemplate` to `warn` and promoted it to `error` on 2025-10-09 after replacing five string concatenations across backend services and tests.
- ✅ **Array syntax aligned**: Enabled `useConsistentArrayType` as `warn` → `error` on 2025-10-08 after updating every `Array<T>` annotation to the project-standard `T[]` shorthand, keeping the repo clean at each step.
- 📅 **Fix window scheduled**: Owners committed to land the `useSimplifiedLogicExpression` fixes by **2025-10-09** so that subsequent rule promotions can begin the same week.
- ✅ **First cleanup merged**: Manually simplified boolean coercions in high-signal services (`TaskService`, `AgentService`, `WorkerApiClient`, and frontend mobile detection hooks) to validate the Biome suggestions before running bulk autofix.

Progress on these action items is tracked in the checklist and the async updates channel described below. New work on additional rules will not start until the current autofix review is complete and merged into `main`. The moment that merge happens, the on-call agent is expected to pull the latest `main`, pick the next highest-priority rule from Phase 1, and begin the enable→fix→promote sequence without waiting for another hand-off.

## Summary Statistics

> **Update (2025-10-08, AM):** Re-ran `bunx biome check --diagnostic-level=error --reporter summary` after promoting `useBlockStatements` to `error`; the tree remains clean at this level. Keep treating the historical breakdown below as the real backlog until each rule is re-enabled and verified.

### Historical breakdown (pre-cleanup)

- **Backend Issues**: 23 errors
- **Frontend Issues**: 26 errors
- **Shared Packages**: 1 error
- **Total Files Affected**: 26 files (520 total files scanned)

### Top Issue Categories

1. **useSimplifiedLogicExpression** (complexity): 50 issues (all auto-fixable)

## Current Snapshot (2025-10-07)

Latest verification:

```
bun run biome:check
```

- Checked 520 files in 1.79s
- No diagnostics reported only because additional rules are still disabled.
- Keep treating the historical `useSimplifiedLogicExpression` list as active work until the follow-up autofix branch lands and the rule is re-promoted.

### Historical snapshot (pre-cleanup)

```
./node_modules/.bin/biome check --diagnostic-level=error --reporter summary
```

- Checked 520 files in 1.88s
- 50 errors emitted, 0 warnings
- All errors map to `lint/complexity/useSimplifiedLogicExpression`
- Detailed per-file counts (26 affected files) come from the same baseline using `--reporter json --max-diagnostics=none` piped
  through `jq`.

### Mainline baseline confirmation

Before updating this plan, fetch and rebase onto the latest `main`, then rerun the Biome summary check to verify the counts:

```bash
git fetch origin main
git rebase origin/main
./node_modules/.bin/biome check --diagnostic-level=error --reporter summary
```

Before the 2025-10-07 cleanup, commit `b0b964c` (latest change to `biome.json`) and head `ccf8146` produced **50 errors across 26 files** with the command above. Keep rerunning it after each batch of fixes so we catch any regressions before promoting additional rules.

### Outstanding Errors by Area

| Area | Files | Errors | Notes |
|------|-------|--------|-------|
| Backend (`apps/backend`) | 11 | 23 | Includes integration tests and several services (auth, file-operations, mcp, preview-proxy, settings, task) |
| Frontend (`apps/frontend`) | 15 | 26 | Concentrated in command form, repository config editor, settings, and task UI components |
| Shared (`packages/types`) | 1 | 1 | `getValidationErrors` helper |

#### Top Files (by error count)

| File | Errors | Notes |
|------|--------|-------|
| `apps/frontend/src/features/commands/components/command-form.tsx` | 8 | Several chained negations in form validation |
| `apps/backend/src/services/file-operations.service.ts` | 6 | Guard clauses around workspace path lookups |
| `apps/frontend/src/features/repositories/components/sections/repository-config-edit-section.tsx` | 5 | Complex form state checks |
| `apps/frontend/src/features/settings/components/system-status-warning.tsx` | 3 | Conditional rendering logic |
| `apps/backend/src/__tests__/complete-workflow.integration.test.ts` | 2 | Timeout guards in integration tests |
| `apps/backend/src/services/mcp.service.ts` | 2 | Null checks on service dependencies |
| `apps/backend/src/services/preview-proxy.service.ts` | 2 | Early returns for missing preview servers |
| `apps/backend/src/services/settings.service.ts` | 2 | Configuration validations |
| `apps/backend/src/services/task.service.ts` | 2 | Task result readiness checks |
| `apps/frontend/src/lib/config/feature-flags.ts` | 2 | Redundant boolean coercions |

### Why `useSimplifiedLogicExpression` matters

- Encourages clearer boolean expressions by avoiding multiple negations.
- Biome can auto-fix each occurrence safely (`FIXABLE` in diagnostics).
- Cleaning this rule now will put the codebase in a good position to start re-enabling previously disabled rules.

## Disabled/Downgraded Rules

The following rules remain disabled or downgraded in `biome.json`. Historical counts from earlier scans are no longer accurate because the codebase has evolved; re-run targeted checks for each rule before re-enabling.

### Style Rules (currently `off`)

| Rule | Status | Suggested Action | Notes |
|------|--------|------------------|-------|
| `useBlockStatements` | error (2025-10-08) | Keep monitoring overrides for tests | Enforces braces around single-line control flow |
| `useConsistentArrayType` | error (2025-10-08) | Continue monitoring generated types for regressions | Enforces shorthand `T[]` syntax |
| `useExplicitLengthCheck` | off | Evaluate auto-fixes in affected collection utilities | Encourages explicit `.length > 0` checks |
| `useTemplate` | off | Audit string concatenations for readability | Prefers template literals |
| `useCollapsedElseIf` | off | Re-enable after `useBlockStatements` | Prevents nested `if` pyramid |
| `useCollapsedIf` | off | Re-enable with targeted tests | Simplifies redundant branches |
| `useFilenamingConvention` | off | Align on kebab-case for new files before enabling | File system conventions |
| `useForOf` | off | Consider effect on performance-sensitive loops | Prefers `for...of` over `forEach` |

### Complexity Rules (currently `off`)

| Rule | Status | Suggested Action | Notes |
|------|--------|------------------|-------|
| `noExcessiveCognitiveComplexity` | off | Collect current complexity scores after `useSimplifiedLogicExpression` fixes | Target complexity ≤ 15 |
| `noForEach` | off | Evaluate viability in async-heavy code | Encourages `for...of` loops |
| `useOptionalChain` | off | Plan staged rollout once optional chaining usage validated | Promotes `?.` and `??` usage |

### Suspicious Rules (downgraded or off)

| Rule | Status | Suggested Action | Notes |
|------|--------|------------------|-------|
| `noExplicitAny` | warn | Track via gradual typing efforts | Use `unknown` + type guards |
| `useAwait` | warn | Audit async functions to remove redundant `async` | Prefer plain Promise returns |
| `noEmptyBlockStatements` | off | Identify legitimate empty catch blocks and document ignores | Avoid silent failures |
| `noGlobalIsNan` | off | Replace with `Number.isNaN` before re-enabling | Aligns with modern JS |

### Performance Rules (currently `off`)

| Rule | Status | Suggested Action | Notes |
|------|--------|------------------|-------|
| `noDelete` | off | Update types to allow `undefined` assignments, then re-enable | Improves hidden class performance |

## Recommended Fix Priority

### Phase 0: Immediate Cleanup (1-2 days)

1. 🔄 **useSimplifiedLogicExpression (50 issues)**
   - Run `biome check --write --max-diagnostics=none` scoped to the files listed above.
   - Review each auto-fix to confirm intent remains unchanged (double-negation vs. grouped conditions).
   - Add regression test coverage where branching logic changes behaviour.

### Phase 1: Prepare Disabled Rules (1 week)

2. 📝 **Collect telemetry for disabled rules**
   - For each `off` rule, spin up a temporary `biome.local.json` (or branch) that raises the rule to `warn` and run Biome to capture counts.
   - Document any intentional exceptions that should remain ignored.

3. 🧭 **Plan refactors for high-impact rules**
   - Prioritise `noExcessiveCognitiveComplexity`, `noDelete`, and `useBlockStatements` based on freshly collected data.
   - Break large refactors into module-sized work items.

### Rule Reactivation Procedure (applies to every disabled rule)

Follow this exact sequence for each rule so that we never mask regressions or mix unrelated fixes in a single commit:

1. **Pick the next rule** from the priority list (start with auto-fixable style rules, then move to complexity/performance).
2. **Create a dedicated branch** (e.g. `chore/biome-reenable-useBlockStatements`). Keep the scope to that single rule.
3. **Flip the rule from `off` → `warn`** in `biome.json` (or scoped override) and run the lint:
   ```bash
   bun run biome:check
   ```
4. **Fix every reported diagnostic**. Use `bun run biome:fix` or targeted `biome check --write` when auto-fixes are available, then hand-polish anything Biome cannot fix.
5. **Re-run the lint command** to confirm the tree is clean at the `warn` level.
6. **Commit the changes** with a rule-specific message (e.g. `chore(biome): enable useBlockStatements as warn`). Include both the code fixes and the config toggle in the same commit.
7. **Raise the rule to `error`**, rerun `bun run biome:check`, and address any additional findings.
8. **Commit again** (e.g. `chore(biome): promote useBlockStatements to error`) once the rule passes at the stricter level.
9. **Open a PR immediately** for review while the context is fresh. Do not queue another rule until the current PR is merged.

### Phase 2: Re-enable with Guard Rails (2-3 weeks)

4. ✅ **Gradually raise rule severities**
   - Apply the **Rule Reactivation Procedure** above for each rule. The key invariants: one rule per branch/PR, lint run after every config change, and a commit each time the tree returns to green.
   - Track progress in the checklist below so we can see exactly which rules have completed both the `warn` and `error` passes.
   - Introduce CI gating (pre-commit + PR checks) once the tree stays green for a week.

## Progress Checklist

Keep this table up to date as rules move through the lifecycle. The **Warn** column should be checked only when the rule is enabled at `warn` severity _and_ the repo is clean. The **Error** column should be checked once the severity is promoted and the tree remains green for at least one verification run.

| Rule | Owner | Warn | Error | Notes |
|------|-------|------|-------|-------|
| useSimplifiedLogicExpression | @frontend-guild | ✅ (2025-10-07) | ✅ (2025-10-07 PM) | Verified after manual cleanup |
| useBlockStatements | @backend-guild | ✅ (2025-10-08) | ✅ (2025-10-08) | Rule re-enabled without additional fixes |
| useConsistentArrayType | @shared-infra | ✅ (2025-10-08) | ✅ (2025-10-08) | Repo converted to preferred `T[]` shorthand |
| useExplicitLengthCheck | TBD | ✅ (2025-10-06) | ✅ (2025-10-06) | Evaluate impact on collection utilities |
| useTemplate | TBD | ✅ (2025-10-09) | ✅ (2025-10-09) | Converted backend concatenations to template literals before promotion |
| useCollapsedElseIf | TBD | ⬜ | ⬜ | Blocked until `useBlockStatements` reaches warn |
| useCollapsedIf | TBD | ⬜ | ⬜ | Schedule after `useCollapsedElseIf` |
| useFilenamingConvention | TBD | ⬜ | ⬜ | Pending decision on conventions for generated files |
| useForOf | TBD | ⬜ | ⬜ | Collect data on hot loops before enabling |
| noExcessiveCognitiveComplexity | TBD | ⬜ | ⬜ | Dependent on fresh complexity scores |
| noForEach | TBD | ⬜ | ⬜ | Requires async iteration review |
| useOptionalChain | TBD | ⬜ | ⬜ | Needs optional chaining adoption guidelines |
| noExplicitAny | TBD | ⬜ | ⬜ | Track via typed API rollout |
| useAwait | TBD | ⬜ | ⬜ | Run audit on redundant async wrappers |
| noEmptyBlockStatements | TBD | ⬜ | ⬜ | Document intentional empties before enabling |
| noGlobalIsNan | TBD | ⬜ | ⬜ | Ensure helper replacements merged |
| noDelete | TBD | ⬜ | ⬜ | Requires type updates for `undefined` assignments |

## Communication & Reporting

- **Weekly Sync**: Add a five-minute Biome segment to the Monday engineering sync to review the checklist and unblock owners.
- **Asynchronous Updates**: Post progress in `#dev-quality` whenever a rule advances a column. Include links to PRs and lint command outputs.
- **Documentation**: When a rule reaches ✅ in both columns, document noteworthy refactor patterns or migration guides in the engineering handbook so future contributors understand the rationale behind the change.

## Risk Mitigation

- **CI Noise**: Limiting each branch/PR to a single rule keeps review scope manageable and prevents noisy cascades when a rule surfaces hundreds of warnings.
- **Regression Safety**: Require targeted tests or snapshots whenever logic branches are simplified by Biome auto-fixes, especially in backend services and shared helpers.
- **Scheduling**: If a sprint introduces high-risk product changes, freeze Biome rule promotions and resume once the release is stable.

5. 🔐 **Tackle type-safety warnings**
   - Schedule pairing sessions to replace `any` usages.
   - For `useAwait`, add ESLint/TypeScript-specific tests to catch redundant async wrappers.

### Phase 3: Stabilise and Monitor (ongoing)

6. 📊 **Automate reporting**
   - Add a weekly `biome check --reporter summary` job to surface new violations early.
   - Keep this plan updated with real counts after each milestone.

7. 🧹 **Document exceptions**
   - Use `biome-ignore` comments sparingly and record justifications in-line plus here.
   - Capture follow-up tickets for any long-lived ignores.

## Auto-Fix Strategy

Many issues can be auto-fixed:

```bash
# Inspect all diagnostics without truncation
./node_modules/.bin/biome check --diagnostic-level=error --max-diagnostics=none

# Auto-fix safe changes (includes useSimplifiedLogicExpression)
./node_modules/.bin/biome check --write --diagnostic-level=error --max-diagnostics=none

# Use the repository script when ready for broader fixes
bun run biome:fix

# Review before committing (required before each "warn" and "error" commit)
git status
git diff
```

### Rules with Auto-Fix

- ✅ `useSimplifiedLogicExpression`
- ✅ `useBlockStatements`
- ✅ `useConsistentArrayType`
- ✅ `useExplicitLengthCheck`
- ✅ `useTemplate`
- ✅ `useOptionalChain`
- ✅ `noUselessConstructor`
- ❌ `noExplicitAny`
- ❌ `noExcessiveCognitiveComplexity`
- ❌ `noDelete`

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
// Rules set to "warn" (should eventually return to "error")
"noExplicitAny": "warn",              // TODO: refresh count after targeted scan
"useAwait": "warn",                    // TODO: refresh count after targeted scan

// Rules set to "off" (should eventually return to "error")
"noExcessiveCognitiveComplexity": "off", // TODO: capture current complexity distribution
"useBlockStatements": "error",         // Restored 2025-10-08 after project-wide brace sweep
"useCollapsedElseIf": "off",          // TODO: verify interaction with block statements
"useCollapsedIf": "off",              // TODO: verify interaction with block statements
"useConsistentArrayType": "error",     // Enforces T[] shorthand (re-enabled 2025-10-08)
"useExplicitLengthCheck": "error",     // TODO: audit collection utilities
"useFilenamingConvention": "off",      // TODO: align repo naming conventions
"useForOf": "off",                     // TODO: check for legitimate `for` loops
"useTemplate": "off",                  // TODO: audit string concatenations
"noForEach": "off",                    // TODO: confirm compatibility with promise flow
"useOptionalChain": "off",             // TODO: evaluate TypeScript target constraints
"noEmptyBlockStatements": "off",       // TODO: document intentional empty blocks
"noGlobalIsNan": "off",                // TODO: replace with Number.isNaN
"noDelete": "off"                      // TODO: ensure types handle `undefined`

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

- [x] Phase 0: Clear all `useSimplifiedLogicExpression` errors
- [ ] Phase 1: Capture fresh counts for disabled rules via temporary config overrides
- [ ] Phase 2: Re-enable easy auto-fix rules as `warn`, then `error`
- [ ] Phase 2: Resolve `noDelete` / `noExcessiveCognitiveComplexity` blockers
- [ ] Phase 3: Remove remaining `warn` levels (e.g. `noExplicitAny`, `useAwait`)
- [ ] Update pre-commit hooks and CI to enforce fully re-enabled rules
- [ ] Document any permanent exceptions with inline `biome-ignore` comments and rationale

### Rule-by-Rule Tracker

Use this table to ensure each rule completes the full enablement loop before starting the next one.

| Rule | Warn enabled & clean | Error enabled & clean | Notes |
|------|----------------------|-----------------------|-------|
| `useSimplifiedLogicExpression` | ✅ (2025-10-07) | ✅ (2025-10-07 PM) | Phase 0 complete |
| `useBlockStatements` | ✅ (2025-10-08) | ✅ (2025-10-08) | Re-enabled globally (tests remain overridden) |
| `useConsistentArrayType` | ☑ | ☑ | Auto-fixable |
| `useExplicitLengthCheck` | ☐ | ☐ | Auto-fixable |
| `useTemplate` | ☐ | ☐ | Auto-fixable |
| `useCollapsedElseIf` | ☐ | ☐ | Follow `useBlockStatements` |
| `useCollapsedIf` | ☐ | ☐ | Follows `useCollapsedElseIf` |
| `useOptionalChain` | ☐ | ☐ | Confirm TS target compatibility |
| `useForOf` | ☐ | ☐ | Check performance-sensitive loops |
| `noForEach` | ☐ | ☐ | Evaluate async impacts |
| `noDelete` | ☐ | ☐ | Coordinate with type updates |
| `noExcessiveCognitiveComplexity` | ☐ | ☐ | Needs refactor plan |
| `noEmptyBlockStatements` | ☐ | ☐ | Document intentional empties |
| `noGlobalIsNan` | ☐ | ☐ | Replace legacy usages |
| `useFilenamingConvention` | ☐ | ☐ | Align naming patterns |

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
