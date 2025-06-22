# Testing Guide

## Test Structure

Tests are organized in `__tests__` directories co-located with the source code:

```
src/features/tasks/
├── components/
│   ├── __tests__/
│   │   ├── task-viewer.test.tsx
│   │   └── task-header.test.tsx
│   └── task-viewer.tsx
├── hooks/
│   ├── __tests__/
│   │   └── use-tasks.test.ts
│   └── use-tasks.ts
```

## Testing Patterns

### Component Testing

```typescript
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { TaskHeader } from '../task-header';

// Create test utilities
const createWrapper = () => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  });
  
  return ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={queryClient}>
      {children}
    </QueryClientProvider>
  );
};

describe('TaskHeader', () => {
  it('displays task metadata', () => {
    render(<TaskHeader />, { wrapper: createWrapper() });
    // Test implementation
  });
});
```

### Hook Testing

```typescript
import { renderHook, waitFor } from '@testing-library/react';
import { useTask } from '../use-tasks';

describe('useTask', () => {
  it('fetches task data', async () => {
    const { result } = renderHook(() => useTask('task-123'), {
      wrapper: createWrapper(),
    });
    
    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });
  });
});
```

### Store Testing

```typescript
import { act } from '@testing-library/react';
import { useTaskUIStore } from '../task-ui-store';

describe('TaskUIStore', () => {
  beforeEach(() => {
    useTaskUIStore.setState(useTaskUIStore.getState());
  });
  
  it('updates selected iteration', () => {
    act(() => {
      useTaskUIStore.getState().setSelectedIteration(2);
    });
    
    expect(useTaskUIStore.getState().selectedIteration).toBe(2);
  });
});
```

## Mocking Strategies

### Mock TanStack Query

```typescript
import { vi } from 'vitest';

vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useTask: vi.fn(() => ({
    data: mockTaskData,
    isLoading: false,
    error: null,
  })),
}));
```

### Mock SSE Connections

```typescript
vi.mock('@/features/tasks/hooks/use-task-sse-connection', () => ({
  useTaskSSEConnection: vi.fn(() => ({
    isConnected: true,
    error: null,
  })),
}));
```

### Mock API Calls with MSW

```typescript
import { http, HttpResponse } from 'msw';
import { setupServer } from 'msw/node';

const server = setupServer(
  http.get('/api/tasks/:id', () => {
    return HttpResponse.json({ id: 'task-123', name: 'Test Task' });
  }),
);

beforeAll(() => server.listen());
afterEach(() => server.resetHandlers());
afterAll(() => server.close());
```

## Test Utilities

Create shared test utilities in `src/test-utils/`:

```typescript
// src/test-utils/render.tsx
export function renderWithProviders(
  ui: React.ReactElement,
  options?: RenderOptions,
) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  
  return render(ui, {
    wrapper: ({ children }) => (
      <QueryClientProvider client={queryClient}>
        {children}
      </QueryClientProvider>
    ),
    ...options,
  });
}
```

## Testing Guidelines

1. **Test File Naming**: Use `.test.ts` or `.test.tsx` extensions
2. **Test Organization**: Group related tests using `describe` blocks
3. **Test Focus**: Test behavior, not implementation details
4. **Mock External Dependencies**: Mock API calls, SSE connections, and external services
5. **Test Coverage**: Aim for >80% coverage on critical paths
6. **Error States**: Always test error handling and edge cases
7. **Async Testing**: Use `waitFor` for async operations
8. **User Interactions**: Use `@testing-library/user-event` for simulating user actions

## Running Tests

```bash
# Run all tests
pnpm test

# Run tests in watch mode
pnpm test:watch

# Run tests with coverage
pnpm test:coverage

# Run tests with UI
pnpm test:ui
```

## Coverage Requirements

- Components: 80% coverage
- Hooks: 90% coverage
- Utilities: 95% coverage
- API functions: 85% coverage

## Common Testing Scenarios

### Testing Loading States

```typescript
it('shows loading skeleton', () => {
  vi.mocked(useTask).mockReturnValue({
    isLoading: true,
    data: undefined,
    error: null,
  });
  
  render(<TaskViewer />);
  expect(screen.getByTestId('task-skeleton')).toBeInTheDocument();
});
```

### Testing Error States

```typescript
it('displays error message', () => {
  vi.mocked(useTask).mockReturnValue({
    isLoading: false,
    data: undefined,
    error: new Error('Failed to fetch'),
  });
  
  render(<TaskViewer />);
  expect(screen.getByText(/failed to fetch/i)).toBeInTheDocument();
});
```

### Testing Real-time Updates

```typescript
it('updates when SSE event received', async () => {
  const { rerender } = render(<EventViewer />);
  
  // Simulate SSE event
  act(() => {
    mockSSEConnection.emit('task:updated', { id: 'task-123' });
  });
  
  await waitFor(() => {
    expect(screen.getByText('Updated')).toBeInTheDocument();
  });
});
```

## Task Feature Testing Strategy

### 1. Unit Tests

#### Hooks (`/features/tasks/hooks/__tests__/`)
- `use-tasks.test.ts` - Test data fetching, caching, error handling
- `use-task-sse-connection.test.ts` - Test SSE connection lifecycle
- `use-task-events-with-streaming.test.ts` - Test event streaming logic

#### Stores (`/features/tasks/stores/__tests__/`)
- `task-ui-store.test.ts` - Test UI state management
- `task-streaming-store.test.ts` - Test streaming state management

#### Components (`/features/tasks/components/__tests__/`)
- `task-viewer.test.tsx` - Test iteration display, message rendering
- `task-header.test.tsx` - Test metadata display, server controls
- `event-viewer.test.tsx` - Test event list, auto-scrolling
- `git-diff-viewer.test.tsx` - Test diff rendering

### 2. Integration Tests

#### Route Tests (`/routes/tasks/__tests__/`)
- `$taskId.test.tsx` - Test full page integration, data loading, error states

### 3. E2E Test Scenarios

1. **Task Creation Flow**
   - Create new task
   - Verify task appears in list
   - Navigate to task detail

2. **Real-time Updates**
   - Open task detail
   - Send message
   - Verify SSE updates
   - Check event stream

3. **Error Handling**
   - Test network failures
   - Test invalid task IDs
   - Test SSE disconnections

## Best Practices

### 1. Test Data Factories

Create factories for consistent test data:

```typescript
// src/test-utils/factories.ts
export const createMockTask = (overrides?: Partial<Task>): Task => ({
  id: 'task-123',
  name: 'Test Task',
  status: 'active',
  created_at: new Date().toISOString(),
  ...overrides,
});
```

### 2. Custom Matchers

Add custom matchers for common assertions:

```typescript
// src/test-utils/matchers.ts
expect.extend({
  toBeLoadingState(received) {
    const pass = received.isLoading && !received.error && !received.data;
    return {
      pass,
      message: () => `expected component to ${pass ? 'not ' : ''}be in loading state`,
    };
  },
});
```

### 3. Test Isolation

- Reset stores between tests
- Clear all mocks after each test
- Use unique IDs for test data

### 4. Performance Testing

```typescript
it('renders large event lists efficiently', () => {
  const manyEvents = Array.from({ length: 1000 }, (_, i) => 
    createMockEvent({ id: `event-${i}` })
  );
  
  const { container } = render(<EventViewer events={manyEvents} />);
  
  // Verify virtualization or pagination is used
  const renderedEvents = container.querySelectorAll('[data-testid="event-item"]');
  expect(renderedEvents.length).toBeLessThan(100);
});
```

## Debugging Tests

### Visual Debugging

```typescript
// Use screen.debug() to see rendered output
screen.debug();

// Use prettyDOM for specific elements
import { prettyDOM } from '@testing-library/react';
console.log(prettyDOM(container));
```

### Testing Library Queries

```typescript
// Prefer these queries in order:
1. getByRole
2. getByLabelText
3. getByPlaceholderText
4. getByText
5. getByDisplayValue
6. getByAltText
7. getByTitle
8. getByTestId // Last resort
```

### Common Issues

1. **Act Warnings**: Wrap state updates in `act()`
2. **Async Issues**: Always use `waitFor` for async operations
3. **Memory Leaks**: Clean up subscriptions and timers
4. **Flaky Tests**: Avoid arbitrary timeouts, use `waitFor` instead