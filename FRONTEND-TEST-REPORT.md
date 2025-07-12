# Frontend Test Optimization Report

## Executive Summary

The frontend tests have been successfully stabilized with memory fixes, but require strategic optimization to improve performance and developer experience. With 16GB memory allocation, tests now complete, but many memory-intensive tests should be categorized differently for optimal CI/CD performance.

## Current Status ✅

### Fixed Issues
- **Memory crashes**: Resolved with 16GB memory allocation
- **Git diff state isolation**: Fixed assertion methods and async handling
- **Task switching behavior**: Added proper `waitFor` assertions for async updates
- **Preview toolbar accessibility**: Added ARIA labels and context mocking
- **React warnings**: Filtered `asChild` props in tooltip mocks

### Remaining Challenges
- High memory consumption (16GB required)
- Some tests still timeout (preview connection tests)
- Complex integration tests running in `test:fast`

## Memory-Intensive Test Analysis

### High Memory Consumers (Should NOT be in `test:fast`)

#### 1. **SSE/Real-time Connection Tests** 🔴
```typescript
// Files to move out of test:fast
src/features/events/contexts/__tests__/global-event-stream-provider.test.tsx
src/features/preview/hooks/__tests__/use-preview-connection.test.tsx
```
**Issues**: 
- Mock EventSource with persistent listeners
- Periodic polling with setInterval (5s intervals)
- Multiple QueryClient instances
- AbortController mocks accumulating

#### 2. **Integration Tests with Multiple Providers** 🔴
```typescript
// Files to move out of test:fast  
src/features/preview/components/__tests__/preview-integration.test.tsx
src/features/tasks/__tests__/task-data-fetching.integration.test.tsx
src/routes/__tests__/tasks.$taskId.test.tsx
```
**Issues**:
- Full component tree rendering
- Multiple context providers stacked
- Heavy DOM manipulation

#### 3. **Large Data Set Tests** 🔴
```typescript
// Files to optimize or move out
src/features/file-viewer/components/__tests__/markdown-preview.test.tsx (Mermaid tests)
src/features/tasks/components/event-viewer/__tests__/event-list-integration.test.tsx
```
**Issues**:
- Mermaid diagram rendering (CPU + memory intensive)
- Large mock event lists (10+ events with complex data)
- Multiple re-renders without cleanup

### Tests That SHOULD Stay in `test:fast` ✅

#### 1. **Pure Unit Tests**
```typescript
// Keep in test:fast - lightweight
src/features/tasks/utils/__tests__/merge-tool-events.test.ts
src/lib/utils/sample.test.ts
src/features/preview/hooks/__tests__/preview-reducer.test.tsx
```

#### 2. **Component Tests with Mocked Dependencies**
```typescript
// Keep in test:fast - properly mocked
src/features/tasks/components/__tests__/task-header.test.tsx
src/features/tasks/components/__tests__/task-card.test.tsx
src/features/settings/components/__tests__/repository-form.test.tsx
```

#### 3. **Hook Tests**
```typescript
// Keep in test:fast - isolated logic
src/features/tasks/hooks/__tests__/use-task-data.test.ts
src/features/repositories/hooks/__tests__/use-repositories.test.tsx
```

## Optimization Plan

### Phase 1: Test Categorization 🎯

#### 1. Update package.json scripts
```json
{
  "scripts": {
    "test": "NODE_OPTIONS='--max-old-space-size=16384' vitest run",
    "test:unit": "NODE_OPTIONS='--max-old-space-size=4096' vitest run --exclude='**/*.integration.test.{ts,tsx}' --exclude='**/*.e2e.test.{ts,tsx}' --exclude='**/*connection*.test.{ts,tsx}' --exclude='**/global-event-stream-provider.test.tsx' --exclude='**/markdown-preview.test.tsx'",
    "test:integration": "NODE_OPTIONS='--max-old-space-size=16384' vitest run '**/*.integration.test.{ts,tsx}' '**/global-event-stream-provider.test.tsx' '**/*connection*.test.{ts,tsx}'",
    "test:fast": "pnpm run test:unit",
    "test:memory-heavy": "NODE_OPTIONS='--max-old-space-size=16384' vitest run '**/markdown-preview.test.tsx' '**/preview-integration.test.tsx'",
    "test:all": "pnpm run test:unit && pnpm run test:integration && pnpm run test:memory-heavy"
  }
}
```

#### 2. Rename Heavy Tests
```bash
# Rename to .integration.test.tsx to auto-exclude from fast tests
mv src/features/events/contexts/__tests__/global-event-stream-provider.test.tsx \
   src/features/events/contexts/__tests__/global-event-stream-provider.integration.test.tsx

mv src/features/preview/hooks/__tests__/use-preview-connection.test.tsx \
   src/features/preview/hooks/__tests__/use-preview-connection.integration.test.tsx
```

### Phase 2: Memory Optimization 🚀

#### 1. Add Test Categories with File Patterns
```typescript
// vitest.config.ts additions
export default defineConfig({
  test: {
    // ... existing config
    pool: 'forks', // Isolate tests in separate processes
    poolOptions: {
      forks: {
        singleFork: true, // Reduce memory by using single fork
      }
    }
  }
})
```

#### 2. Create Memory-Optimized Test Setup
```typescript
// src/test-utils/memory-setup.ts
import { vi, afterEach } from 'vitest'

// Aggressive cleanup for memory-heavy tests
afterEach(() => {
  // Clear DOM completely
  document.body.innerHTML = ''
  document.head.innerHTML = '<title>Test</title>'
  
  // Clear all timers and intervals
  vi.clearAllTimers()
  vi.clearAllMocks()
  
  // Force garbage collection if available
  if (global.gc) {
    global.gc()
  }
})

// Reduce test timeout for memory tests
vi.setConfig({ testTimeout: 30000 })
```

#### 3. Mock Heavy Dependencies More Aggressively
```typescript
// src/test-utils/heavy-mocks.ts
vi.mock('mermaid', () => ({
  default: {
    initialize: vi.fn(),
    render: vi.fn(() => Promise.resolve({ svg: '<svg>mocked</svg>' })),
  }
}))

vi.mock('@git-diff-view/react', () => ({
  DiffView: () => <div data-testid="mock-diff">Mocked Diff</div>,
  parseDiff: vi.fn(() => []),
}))

vi.mock('react-markdown', () => ({
  default: ({ children }: any) => <div data-testid="mock-markdown">{children}</div>
}))
```

### Phase 3: CI/CD Strategy 🏗️

#### 1. GitHub Actions Optimization
```yaml
# .github/workflows/test.yml
name: Tests
on: [push, pull_request]

jobs:
  unit-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: '20'
      - run: pnpm install
      - run: pnpm test:unit  # Fast, low memory
      
  integration-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: '20'
      - run: pnpm install
      - run: pnpm test:integration  # Medium memory
      
  memory-heavy-tests:
    runs-on: ubuntu-latest-4-cores  # Larger runner
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: '20'
      - run: pnpm install  
      - run: pnpm test:memory-heavy  # High memory, parallel
```

#### 2. Pre-commit Hook Strategy
```json
// .husky/pre-push
#!/usr/bin/env sh
. "$(dirname -- "$0")/_/husky.sh"

# Only run unit tests on push - fast feedback
pnpm test:unit
```

### Phase 4: Long-term Optimizations 🔮

#### 1. Test File Size Limits
- Max 200 lines per test file
- Max 5 tests per describe block for heavy tests
- Split large test files into focused suites

#### 2. Memory Monitoring
```typescript
// Add to test setup for monitoring
const getMemoryUsage = () => {
  if (typeof window !== 'undefined' && (window as any).performance?.memory) {
    return (window as any).performance.memory.usedJSHeapSize
  }
  return 0
}

beforeEach(() => {
  const initialMemory = getMemoryUsage()
  console.log(`Test starting with ${(initialMemory / 1024 / 1024).toFixed(2)}MB`)
})
```

#### 3. Selective Test Execution
```bash
# Development workflow
pnpm test:unit                    # Quick feedback (30s)
pnpm test:integration            # Feature testing (2-3min)  
pnpm test:memory-heavy           # Full validation (5-10min)
```

## Expected Outcomes

### Performance Improvements
- **test:fast**: 30-60 seconds (down from 2+ minutes)
- **Memory usage**: 4GB for unit tests (down from 16GB)
- **CI/CD time**: 3-5 minutes total (parallel execution)

### Developer Experience
- Faster feedback loop for common changes
- Reliable test execution without memory crashes
- Clear separation between test types

### Test Reliability
- Reduced flakiness from memory pressure
- Better isolation between test suites  
- Predictable execution times

## Implementation Priority

1. **High Priority**: Update package.json scripts and rename integration tests
2. **Medium Priority**: Add memory monitoring and optimized setup
3. **Low Priority**: Implement CI/CD parallelization and monitoring

This strategy balances test coverage with performance, ensuring developers get fast feedback while maintaining comprehensive validation in CI/CD pipelines.