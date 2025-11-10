# Screenshot Service Refactoring Documentation

This directory contains comprehensive analysis and implementation guides for refactoring the screenshot service architecture.

## Document Index

### 1. REFACTORING_SUMMARY.md (This is the entry point)
**Purpose:** Quick overview of the entire refactoring project
**Read this first** - contains:
- Key findings summary
- Problem statement
- Proposed solution
- Implementation overview
- Impact analysis
- Risk assessment
- Next steps

### 2. SCREENSHOT_REFACTORING_QUICK_REFERENCE.md
**Purpose:** Quick reference guide for developers
**Read this second** - provides:
- Current problems explained clearly
- Service responsibilities breakdown
- Code changes at a glance
- Implementation order
- Testing strategy
- Risk level assessment

### 3. SCREENSHOT_REFACTORING_ANALYSIS.md
**Purpose:** Detailed architectural analysis for review
**Read this for detailed understanding** - includes:
- Complete current architecture overview
- Full service responsibility breakdown
- Data flow diagrams
- Detailed problem identification (6 issues)
- Complete refactoring proposal
- Testing strategy
- Dependency graphs
- Migration checklist
- Risk assessment

### 4. SCREENSHOT_REFACTORING_CODE_GUIDE.md
**Purpose:** Step-by-step implementation guide with code examples
**Read this to implement** - provides:
- Phase 1-5 breakdown
- Complete code snippets for each change
- Exact modifications for each file
- Line-by-line instructions
- Test code examples
- Summary of all changes

## Quick Navigation

**I want to...**

- **Understand what's wrong**: Read REFACTORING_SUMMARY.md sections 1-2
- **See the solution**: Read SCREENSHOT_REFACTORING_QUICK_REFERENCE.md
- **Understand why**: Read SCREENSHOT_REFACTORING_ANALYSIS.md
- **Implement the changes**: Follow SCREENSHOT_REFACTORING_CODE_GUIDE.md
- **Review dependencies**: See SCREENSHOT_REFACTORING_ANALYSIS.md section "Dependency Graph"
- **See risks**: Check SCREENSHOT_REFACTORING_ANALYSIS.md "Risk Assessment"

## Key Takeaways

### The Problem
`ScreenshotOrchestrationService` is a middleman service that:
- Violates Single Responsibility Principle
- Exists only to avoid circular dependencies
- Mixes too many concerns (config, orchestration, persistence, events)

### The Solution
Distribute responsibilities to appropriate services:
- **TaskIterationService**: Config checking, orchestration, task updates, event emission
- **PlaywrightScreenshotService**: Server readiness, waiting, screenshot capture
- **ScreenshotOrchestrationService**: DELETE

### The Impact
- Eliminate ~226 lines (ScreenshotOrchestrationService)
- Add ~150 lines (distributed across other services)
- Net: Delete ~76 lines
- Result: Cleaner, more maintainable architecture

### The Risk
**LOW** - PlaywrightScreenshotService only used by the service we're deleting

## Implementation Phases

1. **Phase 1** (30 min): Update PlaywrightScreenshotService
   - Add 3 new methods
   - Update method signature
   - Add dependency

2. **Phase 2** (30 min): Update TaskIterationService
   - Add 3 new methods
   - Replace service injection
   - Update existing method

3. **Phase 3** (15 min): Update container bindings
   - Remove old service binding
   - Verify new bindings work

4. **Phase 4** (15 min): Delete old code
   - Delete service file
   - Remove exports
   - Search for remaining imports

5. **Phase 5** (45 min): Update and run tests
   - Add new unit tests
   - Update old test mocks
   - Run full test suite

**Total estimated time: 2 hours**

## Getting Started

1. Open REFACTORING_SUMMARY.md
2. Read the "Key Findings" section
3. If you agree with the approach, open SCREENSHOT_REFACTORING_CODE_GUIDE.md
4. Follow Phase 1-5 in order
5. Use the code snippets provided - they're ready to copy-paste
6. Run tests after each phase
7. Commit changes with conventional commits

## Support

If you have questions while implementing:

1. **What does this service do?** 
   - See SCREENSHOT_REFACTORING_ANALYSIS.md "Current Architecture Overview"

2. **Where do I make changes?**
   - See SCREENSHOT_REFACTORING_CODE_GUIDE.md for the exact file and line numbers

3. **How do I test this?**
   - See SCREENSHOT_REFACTORING_CODE_GUIDE.md "Phase 5: Update Tests"

4. **Is this risky?**
   - See REFACTORING_SUMMARY.md "Risk Assessment: LOW"

## References

### Files to Modify
```
✏️ apps/backend/src/features/preview/services/playwright-screenshot.service.ts
✏️ apps/backend/src/features/tasks/services/task-iteration.service.ts
✏️ apps/backend/src/container/container.ts
✏️ apps/backend/src/features/preview/index.ts
✏️ Test files (see CODE_GUIDE.md for specifics)
```

### Files to Delete
```
🗑️ apps/backend/src/features/preview/services/screenshot-orchestration.service.ts
```

### Key Methods to Add

**PlaywrightScreenshotService**
- `ensureServerReady(taskId, repository): Promise<string>`
- `waitForServerReady(url, timeout): Promise<void>` (private)
- Update: `captureIterationScreenshot(taskId, iteration, repository)`

**TaskIterationService**
- `shouldCaptureScreenshot(task): Promise<boolean>` (private)
- `captureScreenshotAndUpdate(task, iteration): Promise<void>`
- `updateTaskAndEmitEvent(task, iteration, screenshotFileId): Promise<void>` (private)
- Update: `performPostProcessing(task, iteration)`

## Questions?

Refer to the detailed analysis documents for:
- Current architecture: SCREENSHOT_REFACTORING_ANALYSIS.md
- Problem details: SCREENSHOT_REFACTORING_ANALYSIS.md "Problems"
- Solution details: SCREENSHOT_REFACTORING_ANALYSIS.md "Proposed Refactoring"
- Implementation code: SCREENSHOT_REFACTORING_CODE_GUIDE.md

