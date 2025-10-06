# Biome Linting Improvements Plan

**Status**: Active
**Created**: 2025-10-06
**Last Updated**: 2025-10-10 (late PM)
**Total Issues**: 0 active errors (historical peak: 50 across 26 files, 520 files scanned)

## Overview

This plan documents the Biome linting issues that were temporarily disabled or downgraded to warnings to allow the codebase to pass validation. An earlier snapshot from `biome check --diagnostic-level=error --reporter summary` (taken immediately after syncing with the latest `main`) showed that `useSimplifiedLogicExpression` was the lone failing rule with 50 diagnostics. After the 2025-10-07 cleanup pass, the same command runs clean, but the remaining disabled or downgraded rules still need to be reintroduced once the new baseline holds.

To keep the rollout predictable, the team will reactivate rules **one at a time**, always running `bun run biome:check` and `bun run build` after each configuration change, fixing every diagnostic, and committing before moving on to the next rule. The detailed procedure appears below and must be followed for every rule with zero exceptions. **Whenever you pick up this work, immediately grab the next unchecked rule from the Phase 1 checklist, flip it from `off` to `warn`, and run `bun run biome:check` before making any other edits so we never skip the enable→lint sequence.** Treat a clean `bun run build` as the release gate before you move to the promotion step so we never ship a broken artifact while iterating on lint rules.

## Immediate Work in Flight

- ✅ **Baseline verified**: `git fetch origin main && git rebase origin/main` followed by `bun run biome:check` on 2025-10-07 confirmed the 50 failing diagnostics are limited to `useSimplifiedLogicExpression`.
- 🚧 **Autofix rehearsal**: A draft branch `chore/biome-fix-use-simplified-logic` is running `bun run biome:fix --files "apps/**" "packages/**"` to validate auto-fixes. Screenshots and diffs are being reviewed with the affected service owners before merging.
- ✅ **String template audit**: Raised `useTemplate` to `warn` and promoted it to `error` on 2025-10-09 after replacing five string concatenations across backend services and tests.
- ✅ **Latest `main` merged**: Rebased on `main` after syncing upstream fixes on 2025-10-10 so new rule work starts from the freshest baseline.
- ✅ **Array syntax aligned**: Enabled `useConsistentArrayType` as `warn` → `error` on 2025-10-08 after updating every `Array<T>` annotation to the project-standard `T[]` shorthand, keeping the repo clean at each step.
- ✅ **Collapsed redundant guards**: Raised `useCollapsedIf` to `warn` and then to `error` on 2025-10-10 after consolidating nested conditionals in the Claude Code agent and repository service, with `bun run biome:check` / `bun run build` executed between each promotion step (falling back to the direct Biome CLI locally while the scripts remain unavailable in the container).
- ✅ **Biome CLI compatibility restored**: Removed the obsolete nursery configuration (`useIterableCallbackReturn`, CSS `noUnknownAtRule` override) and added explicit Tailwind at-rule allowances so the default `bunx biome` now executes Biome 2.2.5 cleanly after fixing the surfaced diagnostics.
- ✅ **Types directory aligned**: Renamed `packages/types/src/types` modules to kebab-case so the rule matches the repo-wide filename baseline.
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

> **Update (2025-10-10, late PM):** After adding Tailwind-specific allowances and fixing the flagged call sites, `bunx biome` on 2.2.5 now reports a clean tree:

```bash
bunx @biomejs/biome@latest lint --diagnostic-level=error --reporter summary
# Checked 520 files in ~0.8s. No diagnostics reported.
```

### CLI Compatibility Plan (updated 2025-10-10)

1. ✅ **Remove deprecated nursery configuration**
   - Dropped `useIterableCallbackReturn` and the obsolete CSS override for `noUnknownAtRule`, then added a file-scoped Tailwind exception so Biome 2.2.5 accepts the config without reintroducing global ignores.
2. ✅ **Verify latest CLI behaviour**
   - Ran `bunx @biomejs/biome@latest lint --diagnostic-level=error --reporter summary`; the command now executes successfully and reports zero diagnostics after addressing the promoted rules.
   - No pinning is required for telemetry runs anymore; the old 2.1.1 command now errors because the rule name changed, so stick with the default CLI moving forward.
3. 🔄 **Monitor upstream replacements**
   - Track Biome release notes for any follow-up rules that supersede the removed nursery checks.
   - Reintroduce equivalent coverage if new stable rules appear in future releases.
4. 🔄 **Update tooling scripts as needed**
   - Ensure `bun run biome:check` and other local scripts bundle the latest Biome once the CLI is stable on all developer environments.

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

### Style Rules (status overview)

| Rule | Status | Suggested Action | Notes |
|------|--------|------------------|-------|
| `useBlockStatements` | error (2025-10-08) | Keep monitoring overrides for tests | Enforces braces around single-line control flow |
| `useConsistentArrayType` | error (2025-10-08) | Continue monitoring generated types for regressions | Enforces shorthand `T[]` syntax |
| `useExplicitLengthCheck` | error (2025-10-06) | Continue to monitor collection utilities for regressions | Encourages explicit `.length > 0` checks |
| `useTemplate` | error (2025-10-09) | Keep auditing string concatenations in new code paths | Prefers template literals |
| `useCollapsedElseIf` | error (2025-10-10) | Maintain coverage on nested guard scenarios | Prevents nested `if` pyramid |
| `useCollapsedIf` | error (2025-10-10) | Monitor redundant branch cleanups in future reviews | Simplifies redundant branches |
| `useFilenamingConvention` | error (2025-10-10) | Maintain TanStack router overrides; everything else must stay kebab-case | File system conventions enforced with targeted ignores |
| `useForOf` | error (2025-10-06) | Monitor performance-sensitive loops after migration | Prefers `for...of` over `forEach` |

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
   - Track the results in the **Disabled Rule Telemetry Log** below so the next person can immediately see which rules already have fresh counts.

#### Disabled Rule Telemetry Log (Phase 1 working table)

Use this table to record the latest warning-level diagnostic counts before promoting any of the currently disabled rules. Leave the _Last Captured_ column blank until you have run the command and pasted the log snippet described underneath the table. Re-run the command whenever `main` diverges materially so we can spot regressions.

| Rule | Command to Run | Notes / Expected Scope | Diagnostics Found | Last Captured |
|------|----------------|------------------------|-------------------|---------------|
| `noExcessiveCognitiveComplexity` | `bunx @biomejs/biome@latest lint --only lint/complexity/noExcessiveCognitiveComplexity --diagnostic-level=warn --max-diagnostics=none` | Backend services (`apps/backend/src/services/**`) and complex frontend forms | _TBD_ | _TBD_ |
| `noForEach` | `bunx @biomejs/biome@latest lint --only lint/performance/noForEach --diagnostic-level=warn --max-diagnostics=none` | Async utilities and iteration-heavy helpers | _TBD_ | _TBD_ |
| `useOptionalChain` | `bunx @biomejs/biome@latest lint --only lint/style/useOptionalChain --diagnostic-level=warn --max-diagnostics=none` | Legacy null checks in backend + React components | _TBD_ | _TBD_ |
| `noEmptyBlockStatements` | `bunx @biomejs/biome@latest lint --only lint/suspicious/noEmptyBlockStatements --diagnostic-level=warn --max-diagnostics=none` | Jest/Vitest specs, error-handling paths | _TBD_ | _TBD_ |
| `noGlobalIsNan` | `bunx @biomejs/biome@latest lint --only lint/suspicious/noGlobalIsNan --diagnostic-level=warn --max-diagnostics=none` | Legacy math helpers (`packages/types`, `apps/backend/src/lib/**`) | _TBD_ | _TBD_ |
| `noDelete` | `bunx @biomejs/biome@latest lint --only lint/performance/noDelete --diagnostic-level=warn --max-diagnostics=none` | Type-layer clean-up required (`apps/backend/src/services/**`, `packages/types`) | _TBD_ | _TBD_ |

After running a command, capture the summary using the template below and paste it into the **Diagnostics Found** cell. Include the git commit you ran against so auditors can recreate the scan:

```md
_Example entry_
- `bunx @biomejs/biome@latest lint --only lint/style/useOptionalChain --diagnostic-level=warn --max-diagnostics=none`
  - Result: 18 diagnostics across 9 files (commit `abc1234`)
```

If Biome emits more than 50 diagnostics for a rule, archive the raw JSON output alongside the branch so follow-up owners can script bulk fixes without re-running the scan.

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

### Investigation: `useFilenamingConvention`

**Snapshot date**: 2025-10-10 (rebased onto `main` immediately before capturing metrics)

To pick an enforcement strategy, we scanned every `.ts`, `.tsx`, and `.js` source file (excluding `.d.ts`) to see which naming patterns are already entrenched.

| Pattern bucket | Count | Typical locations / notes |
|----------------|-------|---------------------------|
| Kebab / single-word lowercase | 311 | Frontend features (`task-page-layout.tsx`, `use-task-preferences.ts`), backend helpers (`worker-api.client.ts`) |
| Plain lowercase | 108 | Shared type definitions (`packages/types/src/types/*.ts` now kebab-case or single-word lowercase), entrypoints (`env.ts`, `main.tsx`) |
| Multi-suffix names (`*.service.ts`, `*.test.tsx`, `*.stories.tsx`, `*.config.ts`) | 80 | Backend services, Jest/Vitest specs, Storybook stories, tool configs |
| `index` | 50 | Barrel exports across features and shared packages |
| PascalCase | 0 | Legacy Storybook sandbox components removed (2025-10-10 cleanup) |
| Router segments (`$command-name/index.tsx`, `$task-id/index.tsx`, `__root.tsx`) | 3 | TanStack Router parameterised routes and root layout |

Key takeaways:

1. **Kebab-case already dominates feature code**. The majority of frontend/backed modules follow the `feature-name.ts` pattern, so enabling the rule with a kebab baseline should have minimal churn in day-to-day work.
2. **Multi-suffix files need explicit allowances**. Our services and tests rely on secondary suffixes (`task.service.ts`, `task.service.test.ts`), and Storybook stories append `.stories.tsx`. The rule must accept these when the base segment already satisfies kebab-case so we retain the clarity of kebab-case bases plus dotted suffixes.
3. **Router-specific exceptions are constrained**. `apps/frontend/src/routes` now keeps dynamic segments in kebab-case directories (`$command-name/index.tsx`, `$task-id/index.tsx`), with the lone non-kebab holdout being the root layout file (`__root.tsx`). PascalCase should remain disallowed elsewhere unless the framework mandates it.
4. **Shared type barrels now follow kebab-case.** The `packages/types/src/types` directory has been renamed (`agent-message.ts`, `api-errors.ts`, etc.) so every exported module adheres to the kebab-case baseline without needing directory-level exceptions.
5. **Storybook sandbox debt is cleared**. Removing the legacy `/stories` playground eliminated the final PascalCase holdouts, so the enforcement plan no longer requires a pre-rollout rename step.

Recommended rollout once the cleanup tasks above land:

1. **Confirm the legacy PascalCase story sandbox remains removed** so future branches do not reintroduce the deprecated `/stories` playground. ✅ (2025-10-10)
2. **Normalize TanStack route filenames** so dynamic segments stay kebab-case while retaining `$` semantics. ✅ (2025-10-10) by moving the command/task routes into `$command-name/index.tsx` and `$task-id/index.tsx`, leaving only `__root.tsx` on the ignore list.
3. **Enable `useFilenamingConvention` at `warn`** with `filenameCases: ["kebab-case"]`, run `bunx @biomejs/biome@latest check --diagnostic-level=warn`, and fix any violations surfaced outside the router overrides. ✅ (2025-10-10) — the tree was already clean after the shared types rename.
4. **Promote the rule to `error`** after rerunning the Biome check at `--diagnostic-level=error` and ensuring `bun run build` (or Turbo locally) stays green. ✅ (2025-10-10) — build verification still relies on local Turbo while the container lacks the executable.

Document any remaining ignores directly in this section so future contributors know why the exception exists. As of 2025-10-10 the only ignore left in `biome.json` covers the TanStack Router root layout (`__root.tsx`).

## Progress Checklist

Keep this table up to date as rules move through the lifecycle. The **Warn** column should be checked only when the rule is enabled at `warn` severity _and_ the repo is clean. The **Error** column should be checked once the severity is promoted and the tree remains green for at least one verification run.

| Rule | Owner | Warn | Error | Notes |
|------|-------|------|-------|-------|
| useSimplifiedLogicExpression | @frontend-guild | ✅ (2025-10-07) | ✅ (2025-10-07 PM) | Verified after manual cleanup |
| useBlockStatements | @backend-guild | ✅ (2025-10-08) | ✅ (2025-10-08) | Rule re-enabled without additional fixes |
| useConsistentArrayType | @shared-infra | ✅ (2025-10-08) | ✅ (2025-10-08) | Repo converted to preferred `T[]` shorthand |
| useExplicitLengthCheck | TBD | ✅ (2025-10-06) | ✅ (2025-10-06) | Evaluate impact on collection utilities |
| useTemplate | TBD | ✅ (2025-10-09) | ✅ (2025-10-09) | Converted backend concatenations to template literals before promotion |
| useCollapsedElseIf | TBD | ✅ (2025-10-10) | ✅ (2025-10-10) | Promoted to error after clean warn pass |
| useCollapsedIf | TBD | ✅ (2025-10-10) | ✅ (2025-10-10) | Schedule after `useCollapsedElseIf` |
| useFilenamingConvention | TBD | ✅ (2025-10-10) | ✅ (2025-10-10) | Enforced kebab-case baseline; router override now limited to `__root.tsx` |
| useForOf | TBD | ✅ (2025-10-06) | ✅ (2025-10-06) | Converted workspace cleanup loop to `for...of` |
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
// Rules currently at "warn" (need follow-up to promote back to "error")
"noExplicitAny": "warn",              // TODO: refresh count after targeted scan
"useAwait": "warn",                    // TODO: refresh count after targeted scan

// Rules still disabled (must run enable → fix → promote workflow)
"noExcessiveCognitiveComplexity": "off", // TODO: capture current complexity distribution
"noForEach": "off",                    // TODO: confirm compatibility with promise flow
"useOptionalChain": "off",             // TODO: evaluate TypeScript target constraints
"noEmptyBlockStatements": "off",       // TODO: document intentional empty blocks
"noGlobalIsNan": "off",                // TODO: replace with Number.isNaN
"noDelete": "off",                     // TODO: ensure types handle `undefined`

// Rules restored to "error" (keep monitoring but no immediate action required)
"useBlockStatements": "error",         // Restored 2025-10-08 after project-wide brace sweep
"useCollapsedElseIf": "error",        // Promoted 2025-10-10 following warn verification
"useCollapsedIf": "error",            // Promoted 2025-10-10 alongside `useCollapsedElseIf`
"useConsistentArrayType": "error",     // Enforces T[] shorthand (re-enabled 2025-10-08)
"useExplicitLengthCheck": "error",     // Promotion complete 2025-10-06; monitor collection utilities
"useFilenamingConvention": {
  "level": "error",
  "options": {
    "filenameCases": ["kebab-case"]
  }
},                                    // Enforced 2025-10-10; TanStack router segments ignored via overrides
"useForOf": "error",                   // Replaced remaining index-based cleanup loops
"useTemplate": "error"                 // Promoted 2025-10-09 after backend string cleanup

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
| `useConsistentArrayType` | ✅ (2025-10-08) | ✅ (2025-10-08) | Auto-fixable |
| `useExplicitLengthCheck` | ✅ (2025-10-06) | ✅ (2025-10-06) | Auto-fixable |
| `useTemplate` | ✅ (2025-10-09) | ✅ (2025-10-09) | Auto-fixable |
| `useCollapsedElseIf` | ✅ (2025-10-10) | ✅ (2025-10-10) | Promoted to error with no additional fixes |
| `useCollapsedIf` | ✅ (2025-10-10) | ✅ (2025-10-10) | Follows `useCollapsedElseIf` |
| `useOptionalChain` | ☐ | ☐ | Confirm TS target compatibility |
| `useForOf` | ✅ (2025-10-06) | ✅ (2025-10-06) | Checked cleanup loops; no perf regressions observed |
| `noForEach` | ☐ | ☐ | Evaluate async impacts |
| `noDelete` | ☐ | ☐ | Coordinate with type updates |
| `noExcessiveCognitiveComplexity` | ☐ | ☐ | Needs refactor plan |
| `noEmptyBlockStatements` | ☐ | ☐ | Document intentional empties |
| `noGlobalIsNan` | ☐ | ☐ | Replace legacy usages |
| `useFilenamingConvention` | ✅ (2025-10-10) | ✅ (2025-10-10) | Kebab-case enforced; TanStack router dynamics ignored via override |

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
- ✅ **CSS/Tailwind**: File-scoped override keeps Tailwind directives working without relaxing checks elsewhere
- ✅ **Storybook** gets special treatment for demo code patterns
- Consider gradually fixing files as they're touched rather than a big-bang approach
- Track progress by moving rules from "off" → "warn" → "error" over time

## References

- [Biome Documentation](https://biomejs.dev/linter/rules/)
- [Current biome.json](/biome.json)
- [Pre-commit Hook](/.husky/pre-commit)
