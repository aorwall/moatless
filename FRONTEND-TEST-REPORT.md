# Frontend Test Failure Report

## Executive Summary

The frontend tests are experiencing significant failures with 35 failing tests across multiple components. The test suite also crashes with a JavaScript heap memory error, indicating serious performance issues.

## Critical Issues

### 1. JavaScript Heap Memory Error
**Severity**: Critical
**Impact**: Tests cannot complete execution
```
FATAL ERROR: Reached heap limit Allocation failed - JavaScript heap out of memory
```
**Solution**: 
- Increase Node.js memory limit for test execution
- Add to package.json test scripts: `NODE_OPTIONS=--max-old-space-size=4096`
- Investigate memory leaks in test setup/teardown

### 2. Segmentation Faults
**Severity**: Critical
**Symptoms**: Random crashes during test execution
**Potential Causes**:
- React 19 compatibility issues with testing library
- Native module conflicts
- Memory corruption in test environment

## Component-Specific Failures

### 1. Git Diff State Isolation (5 failures)
**File**: `src/features/tasks/components/git-diff-viewer/__tests__/git-diff-state-isolation.test.tsx`
**Issues**:
- State mutations between tests
- Incorrect assertion methods (`toContain()` used on strings instead of DOM nodes)
- State not properly isolated between different task contexts

**Fixes Needed**:
```typescript
// Wrong
expect(expandedFiles).toContain('src/file1.ts')

// Correct
expect(Array.from(expandedFiles)).toContain('src/file1.ts')
```

### 2. Task Switching Behavior (2 failures)
**File**: `src/features/tasks/__tests__/task-switching-behavior.test.tsx`
**Issues**:
- Elements not rendering when switching between tasks
- Missing `task2-complex-update` element after task switch
- Likely race condition in state updates

**Fixes Needed**:
- Add `waitFor` assertions for async state updates
- Ensure task switching properly unmounts/remounts components

### 3. Preview Toolbar (11 failures)
**File**: `src/features/preview/components/__tests__/preview-toolbar.test.tsx`
**Issues**:
- Missing accessible roles for buttons
- Inspector toggle button not found
- Context provider not properly mocked

**Fixes Needed**:
- Add proper ARIA labels to buttons
- Ensure PreviewContext is properly provided in tests
- Mock tooltip components to avoid accessibility test failures

### 4. Repository Form (5 failures)
**File**: `src/features/settings/components/__tests__/repository-form.test.tsx`
**Issues**:
- Form validation tests timing out (1000ms+)
- Zod schema validation not working as expected
- Memory issues causing test timeouts

**Fixes Needed**:
- Reduce test timeout expectations
- Mock heavy validation operations
- Fix memory leaks in form component

### 5. Markdown Preview (2 failures)
**File**: `src/features/file-viewer/components/__tests__/markdown-preview.test.tsx`
**Issues**:
- Mermaid diagram re-rendering tests failing
- Memory leak during rapid content changes
- 1000ms+ test execution times

**Fixes Needed**:
- Mock Mermaid rendering to avoid heavy operations
- Add proper cleanup in afterEach hooks

## Common Patterns in Failures

### 1. Missing Context Providers
Many tests fail because they expect contexts that aren't provided:
- `SidebarProvider`
- `PreviewContext`
- `TaskUIProvider`

### 2. Accessibility Issues
- Missing ARIA labels
- Incorrect role queries
- Tooltip components interfering with tests

### 3. Async State Management
- Race conditions in state updates
- Missing `waitFor` assertions
- Improper handling of React 19's concurrent features

### 4. Memory Management
- No cleanup between tests
- Heavy operations not mocked
- Memory leaks in component lifecycle

## Recommended Actions

### Immediate Fixes

1. **Fix Memory Issues**
   ```json
   // package.json
   "test": "NODE_OPTIONS=--max-old-space-size=4096 vitest run",
   "test:fast": "NODE_OPTIONS=--max-old-space-size=4096 vitest run --exclude='**/*.integration.test.{ts,tsx}' --exclude='**/*.e2e.test.{ts,tsx}'"
   ```

2. **Add Test Setup File**
   Create `src/test/setup.ts`:
   ```typescript
   import '@testing-library/jest-dom'
   
   // Mock heavy dependencies
   vi.mock('@git-diff-view/react')
   vi.mock('react-markdown')
   
   // Increase test timeout
   vi.setConfig({ testTimeout: 10000 })
   
   // Add global cleanup
   afterEach(() => {
     cleanup()
     vi.clearAllMocks()
   })
   ```

3. **Fix Context Provider Issues**
   Create test utilities:
   ```typescript
   // src/test/test-utils.tsx
   export function renderWithProviders(ui: React.ReactElement) {
     return render(
       <QueryClient>
         <SidebarProvider>
           <TaskUIProvider>
             {ui}
           </TaskUIProvider>
         </SidebarProvider>
       </QueryClient>
     )
   }
   ```

### Long-term Solutions

1. **Upgrade Testing Dependencies**
   - Ensure React Testing Library is compatible with React 19
   - Update Vitest to latest version
   - Check for known issues with React 19 testing

2. **Implement Test Categories**
   - Separate heavy integration tests
   - Create smoke test suite for CI
   - Add memory profiling to identify leaks

3. **Add Pre-commit Hooks**
   - Run only unit tests on commit
   - Leave integration tests for CI
   - Add memory limit checks

## Test Execution Strategy

### For Development
```bash
# Run only fast unit tests
pnpm test:unit

# Run with increased memory
NODE_OPTIONS=--max-old-space-size=4096 pnpm test
```

### For CI/CD
```bash
# Run tests in batches to avoid memory issues
pnpm test -- --run --reporter=verbose --shard=1/3
pnpm test -- --run --reporter=verbose --shard=2/3
pnpm test -- --run --reporter=verbose --shard=3/3
```

## Conclusion

The frontend tests require significant refactoring to address:
1. Memory management issues
2. React 19 compatibility
3. Context provider setup
4. Async state handling

Priority should be given to fixing the memory issues and critical test infrastructure before addressing individual component test failures.