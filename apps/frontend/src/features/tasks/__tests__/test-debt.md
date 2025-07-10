# Tasks Feature Test Debt

This file tracks technical debt in the tasks feature test suite.

## Skipped Tests

### High Priority
- [ ] **TASK-001**: `event-viewer.test.tsx` - Auto-scroll tests depend on module-level state
  - `scrolls to bottom on first visit`
  - `does not auto-scroll on subsequent visits to same task`
  - **Solution**: Refactor `visitedTasks` to be injectable/resettable for testing

- [ ] **TASK-002**: `event-viewer.test.tsx` - Auto-scroll timing issues
  - `auto-scrolls when new events arrive if at bottom`
  - **Solution**: Use proper async testing utilities and waitFor

- [ ] **TASK-003**: `chat-input-file-search.test.tsx` - All file search tests skipped
  - `should use repository search when in homepage context`
  - `should use task search when in task context`
  - `should show autocomplete results`
  - `should fail to search when repositoryId missing`
  - `should complete file path when selecting`
  - **Solution**: Mock universal file autocomplete hook properly

- [ ] **TASK-004**: `chat-input-expandable.test.tsx` - Multiline input tests
  - `should allow multiline input with Shift+Enter`
  - `should submit on Enter without Shift`
  - **Solution**: Fix keyboard event simulation for userEvent

## Missing Test Coverage

### Components Without Tests (Priority: High)
- [x] **TASK-005**: `task-header.tsx` - Critical UI component ✓ COMPLETED
- [x] **TASK-006**: `task-list.tsx` - Main list view component ✓ COMPLETED
- [x] **TASK-007**: `task-card.tsx` - Individual task display ✓ COMPLETED

### Context Providers Without Tests (Priority: Medium)
- [ ] **TASK-008**: `TaskDataProvider` - Central data management
- [ ] **TASK-009**: `TaskEventsProvider` - Event streaming management
- [ ] **TASK-010**: `TaskUIProvider` - UI state management

### Components Without Tests (Priority: Low)
- [ ] **TASK-011**: `git-diff-viewer/*` - All git diff components
- [ ] **TASK-012**: `server-logs-viewer/*` - Server log display
- [ ] **TASK-013**: Tool call components:
  - `bash-tool-call.tsx`
  - `edit-tool-call.tsx`
  - `write-tool-call.tsx`
  - `read-tool-call.tsx`
  - `ls-tool-call.tsx`
  - Other tool components

### Hooks Without Tests
- [ ] **TASK-014**: `use-auto-scroll.ts`
- [ ] **TASK-015**: `use-claude-code-parser.ts`

## Test Quality Issues

### Tests Needing Improvement
- [ ] **TASK-016**: Inconsistent mock data structures across tests
- [ ] **TASK-017**: Missing error boundary testing
- [ ] **TASK-018**: No accessibility testing for interactive components
- [ ] **TASK-019**: Missing performance tests for large data sets
- [ ] **TASK-020**: No memory leak tests for SSE connections

### Time Display Test Issues
- [ ] **TASK-021**: `task-list.test.tsx` - Time display test failing due to mock data date parsing
- [ ] **TASK-022**: `task-list.test.tsx` - LastMessageAt precedence test failing due to mock data date parsing

## Action Items

1. **Immediate** (This Sprint):
   - Fix TASK-001, TASK-002, TASK-003, TASK-004
   - Add tests for TASK-005, TASK-006, TASK-007

2. **Next Sprint**:
   - Add context provider tests (TASK-008, TASK-009, TASK-010)
   - Fix hook tests (TASK-014, TASK-015)

3. **Future**:
   - Add comprehensive component tests for remaining components
   - Improve test quality issues
   - Add performance and accessibility tests

## Notes

- Tests should follow patterns defined in `/docs/testing-guide.md`
- Use factories from `@/test-utils/factories` for consistent mock data
- Prioritize integration tests over unit tests for components
- All new features must include tests - no exceptions