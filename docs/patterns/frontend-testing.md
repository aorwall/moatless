# Frontend Testing Guide

## Test Structure

Tests are organized in `__tests__` directories co-located with source code:

```
src/features/tasks/
├── components/
│   ├── __tests__/
│   │   ├── task-viewer.unit.test.tsx
│   │   └── task-header.unit.test.tsx
│   └── task-viewer.tsx
├── hooks/
│   ├── __tests__/
│   │   └── use-tasks.unit.test.ts
│   └── use-tasks.ts
```

## Test File Naming Conventions

**Unit Tests** - `*.unit.test.{ts,tsx}` - Fast, isolated tests
**Integration Tests** - `*.integration.test.{ts,tsx}` - Component integration tests  
**E2E Tests** - `*.e2e.test.{ts,tsx}` - Full user journey tests

## Test Commands

```bash
pnpm test           # All tests
pnpm test:unit      # Unit tests only (fastest)
pnpm test:integration # Integration tests only
pnpm test:fast      # Alias for test:unit (git hooks)
pnpm test:coverage  # With coverage report
```

## React 19 Compatibility

```typescript
// Required in src/test-utils/setup.ts
if (!globalThis.IS_REACT_ACT_ENVIRONMENT) {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true
}
```

## Centralized Mock Utilities

### API Hook Mocks

```typescript
import { 
  createMockQuery, 
  createMockLoadingQuery, 
  createMockErrorQuery,
  createMockMutation 
} from '@/test-utils/mocks'

// Mock successful data
vi.mocked(useTask).mockReturnValue(createMockQuery(taskData))

// Mock loading state
vi.mocked(useTask).mockReturnValue(createMockLoadingQuery())

// Mock error state
vi.mocked(useTask).mockReturnValue(
  createMockErrorQuery(new Error('Failed to fetch'))
)

// Mock mutation
vi.mocked(useCreateTask).mockReturnValue(
  createMockMutation({ mutate: mockMutate })
)
```

### SSE/WebSocket Mocks

```typescript
import { createMockSSEConnection } from '@/test-utils/mocks'

const { mockEventSource, EventSourceSpy, restore } = createMockSSEConnection()

beforeEach(() => {
  global.EventSource = EventSourceSpy
})

afterEach(() => {
  restore()
})

it('handles SSE updates', () => {
  render(<ComponentWithSSE />)
  
  act(() => {
    mockEventSource.triggerOpen()
    mockEventSource.simulateData({ type: 'update', data: 'test' })
  })
})
```

### UI Component Mocks

```typescript
import { mockResizableComponents, mockDialogComponents } from '@/test-utils/mocks'

vi.mock('@/lib/components/ui/resizable', () => mockResizableComponents())
vi.mock('@/lib/components/ui/dialog', () => mockDialogComponents())
```

### Provider Wrappers

```typescript
import { createQueryWrapper } from '@/test-utils/mocks'

const { wrapper } = createQueryWrapper()
render(<Component />, { wrapper })
```

## Migration Guide

### Before (Old Pattern)
```typescript
vi.mocked(useTask).mockReturnValue({
  data: taskData,
  isLoading: false,
  error: null,
  // ... many more properties
} as any)
```

### After (New Pattern)
```typescript
import { createMockQuery } from '@/test-utils/mocks'
vi.mocked(useTask).mockReturnValue(createMockQuery(taskData))
```

## Key Testing Principles

### 1. Strategic Mocking - Decision Tree

```
Is it an external dependency?
├─ Yes → Is it one of these?
│   ├─ HTTP API call (fetch, axios) → Mock with createMockQuery()
│   ├─ SSE/WebSocket → Mock with createMockSSEConnection()
│   ├─ Browser API (localStorage, etc.) → Mock with vi.stubGlobal()
│   └─ Third-party service → Mock the service module
└─ No → Is it a UI library component?
    ├─ Yes → Does it have these characteristics?
    │   ├─ Requires DOM measurements → Mock it
    │   ├─ Has complex internal state → Mock it
    │   ├─ Uses ResizeObserver/IntersectionObserver → Mock it
    │   └─ Simple display component → DO NOT mock
    └─ No → DO NOT mock (test real integration)
```

### Concrete Mocking Rules

**ALWAYS Mock These (External Dependencies)**:
- `fetch()`, `axios`, or any HTTP client
- `EventSource` for SSE connections
- `WebSocket` connections
- `window.localStorage`, `window.sessionStorage`
- External authentication services
- File upload/download APIs
- Browser notifications API

**ALWAYS Mock These UI Libraries**:
- `@/lib/components/ui/resizable` - Uses ResizeObserver
- `react-virtualized` or similar - Complex viewport calculations
- `react-beautiful-dnd` - Drag state management
- Chart/graph libraries - Canvas/SVG rendering
- Rich text editors - Complex DOM manipulation

**NEVER Mock These**:
- Components in the same feature folder
- React hooks from the same codebase
- Simple UI components (Button, Input, Card)
- Utility functions
- Type definitions or constants

### 2. Test Behavior, Not Implementation

**Concrete Rule**: Test what users see and interact with, not how the code works internally.

**Test These (Observable Behavior)**:
- DOM elements rendered: `screen.getByText()`, `screen.getByRole()`
- User interactions: `userEvent.click()`, `userEvent.type()`
- Visual states: presence/absence of elements
- Text content displayed to users
- Form submission results
- Navigation changes

**Do NOT Test These (Implementation Details)**:
- Component state variables
- Hook return values directly
- Private methods
- Redux store state directly
- CSS class names (unless they affect behavior)
- Function call counts (unless testing error retry logic)

```typescript
// ❌ BAD: Testing implementation details
expect(component.state.isLoading).toBe(true)
expect(mockSetState).toHaveBeenCalledWith({ open: true })
expect(wrapper.find('.modal-wrapper').hasClass('active')).toBe(true)

// ✅ GOOD: Testing observable behavior
expect(screen.getByTestId('loading-spinner')).toBeInTheDocument()
expect(screen.getByRole('dialog')).toBeInTheDocument()
expect(screen.queryByText('Modal Content')).toBeVisible()
```

### 3. Integration Testing Pattern

```typescript
// Mock only external dependencies
vi.mock('@/features/tasks/hooks/use-tasks')
vi.mock('@/features/tasks/hooks/use-task-sse-connection')

// Test real component integration
it('displays task data through component hierarchy', () => {
  vi.mocked(useTask).mockReturnValue(createMockQuery(taskData))
  render(<TaskDetail />)
  
  // Verify real data flow
  expect(screen.getByText(taskData.title)).toBeInTheDocument()
  expect(screen.getByText(taskData.iterations[0].message)).toBeInTheDocument()
})
```

## Common Testing Patterns

### Loading States
```typescript
it('shows loading skeleton', () => {
  vi.mocked(useTask).mockReturnValue(createMockLoadingQuery())
  render(<TaskViewer />)
  expect(screen.getByTestId('task-skeleton')).toBeInTheDocument()
})
```

### Error States
```typescript
it('displays error message', () => {
  vi.mocked(useTask).mockReturnValue(
    createMockErrorQuery(new Error('Network error'))
  )
  render(<TaskViewer />)
  expect(screen.getByText(/Network error/)).toBeInTheDocument()
})
```

### Real-time Updates
```typescript
it('updates when SSE event received', async () => {
  const { mockEventSource } = createMockSSEConnection()
  render(<EventViewer />)
  
  act(() => {
    mockEventSource.simulateData({ type: 'task:updated', data: newData })
  })
  
  await waitFor(() => {
    expect(screen.getByText(newData.status)).toBeInTheDocument()
  })
})
```

## Test Quality Standards

### Mandatory Test Requirements

**Every test MUST**:
1. Assert on actual behavior (no `expect(true).toBe(true)`)
2. Use centralized mock utilities from `@/test-utils/mocks`
3. Clean up after itself (no test pollution)
4. Complete within 1 second (move to integration if slower)

**Test Coverage Minimums**:
- User-facing components: ≥80% coverage
- Hooks with business logic: ≥90% coverage
- Utility functions: 100% coverage
- Pure functions: 100% coverage

**Forbidden Patterns**:
- `.skip()` or `.only()` in committed code
- `console.log()` statements (use `debug()` from testing-library)
- `any` type assertions in expectations
- Hardcoded timeouts > 1000ms
- Direct module state mutation

## Troubleshooting

### Common Issues

#### "Cannot read properties of undefined" in Mocked Hook
```typescript
// ❌ BAD: Incomplete mock
vi.mocked(useTask).mockReturnValue({ data: taskData } as any)

// ✅ GOOD: Complete mock
vi.mocked(useTask).mockReturnValue(createMockQuery(taskData))
```

#### Provider Missing Error
```typescript
import { createQueryWrapper } from '@/test-utils/mocks'
const { wrapper } = createQueryWrapper()
render(<Component />, { wrapper })
```

#### Async Timing Issues
```typescript
// Always use waitFor for async operations
await waitFor(() => {
  expect(screen.getByText('Success')).toBeInTheDocument()
})
```

#### SSE Connection Cleanup
```typescript
let mockConnection: ReturnType<typeof createMockSSEConnection>

beforeEach(() => {
  mockConnection = createMockSSEConnection()
  global.EventSource = mockConnection.EventSourceSpy
})

afterEach(() => {
  mockConnection.restore()
})
```

## Test Organization Best Practices

```typescript
describe('ComponentName', () => {
  describe('Rendering', () => {
    it('renders with default props', () => {})
    it('handles missing data gracefully', () => {})
  })
  
  describe('User Interactions', () => {
    it('handles click events', () => {})
    it('processes form submissions', () => {})
  })
  
  describe('Error Handling', () => {
    it('displays error message on failure', () => {})
    it('provides retry mechanism', () => {})
  })
})
```

## Factory Functions

```typescript
// src/test-utils/factories/task.factory.ts
export const createMockTask = (overrides?: Partial<Task>): Task => ({
  id: 'task-' + Math.random().toString(36).substr(2, 9),
  title: 'Default Task Title',
  status: 'active',
  created_at: new Date().toISOString(),
  ...overrides,
})
```

## Performance Optimization

### Mock Heavy Components
```typescript
vi.mock('@/features/charts/heavy-chart', () => ({
  HeavyChart: () => <div data-testid="chart">Chart</div>
}))
```

### Test Performance Guidelines
For detailed test categorization by speed and backend-specific performance tiers, see [Backend Testing Guidelines](../guidelines/backend-testing.md).

### Strategic Mocking Framework
This guide follows the [Strategic Mocking Pattern](../guidelines/strategic-mocking.md) for balanced test isolation and integration. See that document for:
- Mocking decision frameworks
- Contract testing patterns
- Integration test strategies
- Mock factory patterns

## Quick Reference

### Mock Utilities Exports
```typescript
// API Hooks
export { createMockQuery }         // Success state
export { createMockLoadingQuery }  // Loading state  
export { createMockErrorQuery }    // Error state
export { createMockMutation }      // Mutations

// SSE/WebSocket
export { createMockSSEConnection } // SSE connection
export { MockEventSource }         // EventSource mock
export { MockWebSocket }           // WebSocket mock

// UI Components
export { mockResizableComponents }
export { mockDialogComponents }
export { mockSelectComponents }
export { mockTabsComponents }

// Providers
export { createQueryWrapper }
export { createAllProvidersWrapper }
```

### Testing Checklist
- [ ] Use proper naming convention (*.unit.test.tsx or *.integration.test.tsx)
- [ ] No meaningless assertions
- [ ] Use centralized mocks from @/test-utils/mocks
- [ ] Test all states (loading, error, success)
- [ ] Clean up after tests (timers, subscriptions)
- [ ] Test runs in appropriate time for its category