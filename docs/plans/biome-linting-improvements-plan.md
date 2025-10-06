# Biome Linting Improvements Plan

**Status**: Active - `noExcessiveCognitiveComplexity` In Progress (Batch 1 Complete)
**Last Updated**: 2025-10-06
**Current Focus**: Completed Batch 1 (high complexity 30+). Continue with Batch 2 (medium-high complexity 20-29)

## Overview

This plan tracks the systematic re-enablement of Biome linting rules. Previously disabled rules are being enabled at `error` level, fixed, and committed one rule at a time.

## Current Work: noExcessiveCognitiveComplexity

**Status**: Rule enabled at `error` level, Batch 1 complete (7 of 23 functions fixed)

### Progress Summary

**Completed Refactorings** (7/23):
- ✅ `claude-code-agent.ts:run()` - 69 → <15 (extracted 11 helper methods)
- ✅ `message.service.ts:readMessagesFromClaudeDirectory()` - 69 → <15 (extracted 6 helper methods)
- ✅ `message.service.ts:transformAgentMessagesToMessages()` - 50 → <15 (extracted 5 helper methods)
- ✅ **Batch 1 Complete** (2025-10-06):
  - `message.service.ts:transformAgentAssistantMessage()` - 47 → <15 (extracted 7 helper methods)
  - `tasks.ts:deployRoute` handler - 46 → <15 (extracted 7 helper functions)
  - `claude.service.ts:getSessionUsage()` - 37 → <15 (extracted 10 helper methods)
  - `workspace.ts:buildFileTree()` - 32 → <15 (extracted 6 helper methods)

**Remaining Work** (16 functions):

| File | Function | Complexity | Priority |
|------|----------|------------|----------|
| `agent.service.ts` | `executeAgent()` | 29 | Medium |
| `worker-api-client.ts` | `transformDates()` | 29 | Medium |
| `docker-environment.ts` | `waitForContainerReady()` | 28 | Medium |
| `workspace.ts` | `executeCommandsWithStream()` | 25 | Medium |
| `claude.service.ts` | `calculateHistoricalP90Limit()` | 24 | Medium |
| `docker.service.ts` | `createContainer()` | 23 | Medium |
| `events.ts` | Stream handler (line 15) | 22 | Medium |
| `local-environment.ts` | `runSetupCommands()` | 21 | Medium |
| `claude.service.ts` | `createSessionBlock()` | 21 | Medium |
| `worker-app.ts` | Proxy handler (line 357) | 20 | Low |
| `claude.service.ts` | `findClaudeBinDir()` | 20 | Low |
| `claude.service.ts` | `validateClaudePaths()` | 20 | Low |
| `claude-utils.ts` | `findClaudeBinPath()` | 18 | Low |
| `claude.service.ts` | `getUsage()` | 18 | Low |
| `file-operations.service.ts` | `readFile()` | 17 | Low |
| `docker-environment.ts` | `setup()`, `runCommand()` | 16 each | Low |
| `local-environment.ts` | `startServer()` | 16 | Low |
| `preview-server.ts` | `start()` | 16 | Low |

### Refactoring Strategy

**Pattern to Follow** (demonstrated in completed work):

1. **Keep public methods at the top** - Main entry point stays simple and readable
2. **Extract helper methods in logical flow order** - Helpers appear below in the order they're called
3. **Use descriptive helper names** - Each helper does one clear thing
4. **Preserve all functionality** - Behavior must remain identical

**Example Pattern** (from `claude-code-agent.ts`):
```typescript
// Public method - simple, delegates to helpers
async *run(message: string, options: Options): AsyncGenerator<Event> {
  const logger = this.createLogger(options)
  this.logRunStart(logger, options)

  try {
    const sdkOptions = this.buildSDKOptions(options)
    yield* this.processSDKMessages(message, sdkOptions, logger)
  } catch (error) {
    yield* this.handleError(logger, error)
  }
}

// Private helpers follow in logical order
private createLogger(options: Options) { ... }
private logRunStart(logger, options) { ... }
private buildSDKOptions(options) { ... }
private *processSDKMessages(message, options, logger) { ... }
private *handleError(logger, error) { ... }
```

### Batch Refactoring Plan

**Batch 1: High Complexity (Complexity 30+)** - 4 functions
- Focus on most complex first to reduce overall codebase complexity
- Files: `message.service.ts`, `tasks.ts`, `claude.service.ts`, `workspace.ts`

**Batch 2: Medium-High (Complexity 20-29)** - 9 functions
- Group by file where possible (e.g., all `claude.service.ts` functions together)
- Maintain file structure: public methods → private helpers

**Batch 3: Low Complexity (Complexity 16-19)** - 7 functions
- These are just barely over the limit, usually need 1-2 helper extractions
- Quick wins to complete the rule enablement

### Verification Steps

After each batch:
```bash
# 1. Check complexity is resolved
bunx @biomejs/biome@latest lint apps/backend \
  --only lint/complexity/noExcessiveCognitiveComplexity \
  --diagnostic-level=error

# 2. Verify types still pass
cd apps/backend && bun run type:check

# 3. Verify build works
bun run build
```

## Rule Reactivation Procedure

Standard workflow for enabling any disabled rule:

1. **Enable rule at `error`** in `biome.json`
2. **Run `bun run biome:check`** to see diagnostics
3. **Fix all violations** (use `bun run biome:fix` for auto-fixable rules)
4. **Verify build passes**: `bun run build`
5. **Commit once** with config change + all fixes together
6. **Update this plan**: Mark rule complete

## Previously Completed Rules

- ✅ `useSimplifiedLogicExpression` - Simplified 50 boolean expressions (2025-10-07)
- ✅ `useBlockStatements` - Added braces to single-line control flow (2025-10-08)
- ✅ `useConsistentArrayType` - Converted to `T[]` shorthand (2025-10-08)
- ✅ `useExplicitLengthCheck` - Changed to `.length > 0` pattern (2025-10-06)
- ✅ `useTemplate` - Converted string concatenation to templates (2025-10-09)
- ✅ `useCollapsedElseIf` - Collapsed nested if/else chains (2025-10-10)
- ✅ `useCollapsedIf` - Merged redundant conditionals (2025-10-10)
- ✅ `useFilenamingConvention` - Enforced kebab-case (2025-10-10)
- ✅ `useForOf` - Replaced index loops with for...of (2025-10-06)

## Remaining Disabled Rules

### Complexity Rules (1 in progress, 2 disabled)

| Rule | Status | Action Plan | Auto-fix |
|------|--------|-------------|----------|
| `noExcessiveCognitiveComplexity` | 🚧 error (3/23 done) | **IN PROGRESS** - See batch plan above. Refactor 20 remaining functions by extracting helper methods. Target completion: complete all 3 batches. | ❌ |
| `noForEach` | ❌ off | Enable, scan codebase for async forEach usage patterns. Replace with `for...of` loops. May require Promise.all() for parallel operations. | ✅ |
| `useOptionalChain` | ❌ off | Enable, auto-fix nested null checks (`a && a.b && a.b.c` → `a?.b?.c`). Verify TS target supports optional chaining. | ✅ |

### Suspicious Rules (2 at warn, 2 disabled)

| Rule | Status | Action Plan | Auto-fix |
|------|--------|-------------|----------|
| `noExplicitAny` | ⚠️ warn | Gradual migration: promote to `error`, fix one module at a time. Replace `any` with proper types or `unknown` + type guards. Track progress per module. | ❌ |
| `useAwait` | ⚠️ warn | Promote to `error`. Audit all async functions, remove unnecessary `async` keyword where function just returns Promise directly. | ❌ |
| `noEmptyBlockStatements` | ❌ off | Enable, document intentional empty blocks with comments explaining why (e.g., `catch {} // Intentionally ignore error`). Add `biome-ignore` for legitimate cases. | ❌ |
| `noGlobalIsNan` | ❌ off | Enable, auto-fix all `isNaN()` → `Number.isNaN()`. Modern JS best practice for NaN checking. | ✅ |

### Performance Rules (1 disabled)

| Rule | Status | Action Plan | Auto-fix |
|------|--------|-------------|----------|
| `noDelete` | ❌ off | Enable after type refactor. Update interfaces to make properties optional, use `obj.prop = undefined` instead of `delete obj.prop`. Improves V8 hidden class performance. | ❌ |

**Legend**: ✅ Complete | 🚧 In Progress | ⚠️ Warning Level | ❌ Disabled

### Next Priority After noExcessiveCognitiveComplexity

1. **noForEach** (complexity) - Auto-fixable, straightforward replacement
2. **useOptionalChain** (complexity) - Auto-fixable, improves readability
3. **noGlobalIsNan** (suspicious) - Auto-fixable, quick win
4. **useAwait** (suspicious) - Promote from warn to error, manual fixes
5. **noEmptyBlockStatements** (suspicious) - Requires case-by-case review
6. **noExplicitAny** (suspicious) - Large effort, do incrementally
7. **noDelete** (performance) - Requires type system changes first

## References

- [Biome Documentation](https://biomejs.dev/linter/rules/)
- [Current biome.json](/biome.json)
