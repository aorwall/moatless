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

## React 19 Compatibility

This project uses React 19 with @testing-library/react. To ensure compatibility:

### Test Setup Configuration

The test setup file (`src/test-utils/setup.ts`) includes React 19 compatibility:

```typescript
// React 19 compatibility: Set the act environment flag
// This tells React Testing Library that we're in a test environment
if (!globalThis.IS_REACT_ACT_ENVIRONMENT) {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true
}
```

This flag is required for React 19 to properly handle the `act` function in tests. Without it, you'll encounter "React.act is not a function" errors.

### Important Notes

1. **@testing-library/react v16.3.0** works with React 19 when the environment flag is set
2. **No downgrade needed** - Keep React 19 and use the compatibility flag
3. **act() is built into React 19** - The testing library uses React's native act implementation

## Testing Strategy

### Layered Testing Approach

This project uses a layered testing strategy that balances coverage, speed, and reliability:

1. **Unit Tests** - Test individual components and hooks in isolation
2. **Integration Tests** - Test components working together with minimal mocking
3. **Contract Tests** - Verify component interfaces and dependency contracts
4. **E2E Tests** - Test complete user journeys (planned)

### Strategic Mocking Philosophy

The key principle: **Mock at the boundaries, test the behavior**.

#### ✅ WHAT TO MOCK

1. **External Dependencies**
   - API calls and data fetching hooks
   - SSE connections and real-time events
   - Third-party libraries
   - Browser APIs

2. **Complex Infrastructure**
   - UI libraries that don't affect business logic (resizable panels, etc.)
   - Router and navigation
   - Error boundaries and skeleton components

3. **Unstable Dependencies**
   - Components that require complex setup (providers, contexts)
   - Components with side effects (file uploads, etc.)

#### ❌ WHAT NOT TO MOCK

1. **Business Logic Components**
   - Feature components when testing integration
   - Components that contain your business logic
   - Components under direct test

2. **Simple UI Components**
   - Buttons, inputs, basic form elements
   - Layout components without complex logic

3. **Your Own Code**
   - Utility functions
   - Custom hooks (test them separately)

### Example: Route Integration vs Unit Testing

#### ❌ Over-mocked Route Test (Anti-pattern)
```typescript
// This misses integration bugs!
vi.mock('@/features/tasks/components/task-viewer', () => ({
  TaskViewer: () => <div>Mocked Task Viewer</div>
}));
vi.mock('@/features/tasks/components/task-chat-input', () => ({
  TaskChatInput: () => <div>Mocked Chat Input</div>
}));

// Test tells us nothing about real integration
it('renders components', () => {
  render(<TaskDetail />);
  expect(screen.getByText('Mocked Task Viewer')).toBeInTheDocument();
});
```

#### ✅ Strategic Integration Test
```typescript
// Mock only external dependencies
vi.mock('@/features/tasks/hooks/use-tasks');
vi.mock('@/features/tasks/hooks/use-task-sse-connection');

// Test real component integration
it('displays task data through component hierarchy', () => {
  const taskData = createMockTaskWithIterations({
    title: 'Integration Test Task'
  }, 2);
  
  vi.mocked(useTask).mockReturnValue({
    data: taskData,
    isLoading: false,
    error: null
  });

  render(<TaskDetail />);

  // Verify real data flow through real components
  expect(screen.getByText('Integration Test Task')).toBeInTheDocument();
  expect(screen.getByText(taskData.iterations[0].initial_message)).toBeInTheDocument();
  
  // Test real interactions
  const diffTab = screen.getByRole('tab', { name: /diff/i });
  await user.click(diffTab);
  expect(mockSetRightPanelView).toHaveBeenCalledWith('diff');
});
```

### Benefits of Strategic Mocking

#### Integration Testing Catches:
- **Interface Mismatches**: When component props change
- **Data Flow Issues**: When context or props aren't passed correctly
- **Breaking Changes**: When child components break parent functionality
- **Performance Issues**: When components don't handle large datasets
- **User Experience Issues**: When interactions don't work as expected

#### Example: Real Bug Caught by Integration Test
```typescript
// This integration test would catch the bug:
it('handles task status changes', () => {
  const runningTask = createMockTask({ is_running: true });
  vi.mocked(useTask).mockReturnValue({ data: runningTask });
  
  render(<TaskDetail />);
  
  // Would fail if TaskViewer doesn't show processing state
  expect(screen.getByText('Processing...')).toBeInTheDocument();
  
  // Would fail if ChatInput doesn't disable during running state
  const sendButton = screen.getByRole('button', { name: /send/i });
  expect(sendButton).toBeDisabled();
});
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

## Strategic Mocking Patterns

### 1. External Dependencies

#### API Hooks
```typescript
// Mock the entire hooks module
vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useTask: vi.fn(),
  useTasksList: vi.fn(),
  useCreateTask: vi.fn(),
}));

// In tests, provide specific return values
vi.mocked(useTask).mockReturnValue({
  data: createMockTask(),
  isLoading: false,
  error: null,
});
```

#### SSE Connections
```typescript
vi.mock('@/features/tasks/hooks/use-task-sse-connection', () => ({
  useTaskSSEConnection: vi.fn(() => ({
    isConnected: true,
    error: null,
    disconnect: vi.fn(),
  }))
}));
```

### 2. Complex UI Libraries

#### Resizable Panels
```typescript
// Mock complex layout components that don't affect business logic
vi.mock('@/lib/components/ui/resizable', () => ({
  ResizablePanel: ({ children }: any) => <div>{children}</div>,
  ResizablePanelGroup: ({ children }: any) => <div>{children}</div>,
  ResizableHandle: () => <div data-testid="resize-handle" />,
}));
```

### 3. Router and Navigation
```typescript
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'test-task-id' }),
  useNavigate: () => vi.fn(),
  createFileRoute: (path: string) => (config: any) => ({
    ...config,
    useParams: () => ({ taskId: 'test-task-id' }),
  }),
}));
```

### 4. Error Boundaries and Skeletons
```typescript
// Mock complex loading/error components to simplify testing
vi.mock('@/features/tasks/components/task-skeletons', () => ({
  TaskDetailSkeleton: () => <div data-testid="task-skeleton">Loading...</div>,
}));

vi.mock('@/lib/components/ui/error-alert', () => ({
  ErrorAlert: ({ error, title }: any) => (
    <div data-testid="error-alert">
      <h2>{title}</h2>
      <p>{error.message}</p>
    </div>
  ),
}));
```

## Mocking Anti-Patterns

### ❌ Over-Mocking Anti-Patterns

#### Don't Mock Everything
```typescript
// ❌ BAD: Over-mocking loses integration value
vi.mock('@/features/tasks/components/task-viewer');
vi.mock('@/features/tasks/components/task-chat-input');
vi.mock('@/features/tasks/components/event-viewer');
vi.mock('@/features/tasks/components/git-diff-viewer');

// Now you're just testing mocks!
```

#### Don't Mock Simple Components
```typescript
// ❌ BAD: Mocking simple UI components
vi.mock('@/lib/components/ui/button', () => ({
  Button: ({ children, onClick }: any) => (
    <button onClick={onClick}>{children}</button>
  )
}));

// ✅ GOOD: Let real buttons render - they're simple!
// No mock needed
```

#### Don't Create Fake Implementations
```typescript
// ❌ BAD: Creating elaborate fake implementations
vi.mock('@/features/tasks/components/task-viewer', () => ({
  TaskViewer: ({ onIterationSelect }: any) => {
    const [selected, setSelected] = useState(0);
    return (
      <div>
        {Array.from({length: 3}).map((_, i) => (
          <button 
            key={i} 
            onClick={() => {
              setSelected(i);
              onIterationSelect?.(i);
            }}
          >
            Iteration {i}
          </button>
        ))}
      </div>
    );
  }
}));

// ✅ GOOD: Test the real component!
```

### ✅ Good Mocking Patterns

#### Mock External Dependencies
```typescript
// ✅ GOOD: Mock data fetching
vi.mock('@/features/tasks/hooks/use-tasks', () => ({
  useTask: vi.fn(() => ({
    data: mockTaskData,
    isLoading: false,
    error: null,
  })),
}));
```

#### Mock Complex Infrastructure
```typescript
// ✅ GOOD: Mock complex layout systems
vi.mock('@/lib/components/ui/resizable', () => ({
  ResizablePanel: ({ children }: any) => <div>{children}</div>,
  ResizablePanelGroup: ({ children }: any) => <div>{children}</div>,
}));
```

#### Provide Realistic Mock Data
```typescript
// ✅ GOOD: Realistic mock data
const mockTask = createMockTaskWithIterations({
  task_id: 'task-123',
  title: 'Real Looking Task Title',
  status: 'active',
  iterations: [
    {
      iteration_id: 'iter-1',
      initial_message: 'First iteration message',
      status: 'completed'
    },
    {
      iteration_id: 'iter-2', 
      initial_message: 'Second iteration message',
      status: 'running'
    }
  ]
});

vi.mocked(useTask).mockReturnValue({
  data: mockTask,
  isLoading: false,
  error: null
});
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

### Core Principles

1. **Test File Naming**: Use `.test.ts` or `.test.tsx` extensions
2. **Test Organization**: Group related tests using `describe` blocks
3. **Test Focus**: Test behavior, not implementation details
4. **Integration First**: Prefer integration tests for routes and complex features
5. **Strategic Mocking**: Mock only external dependencies and complex infrastructure
6. **Test Coverage**: Aim for >80% coverage on critical paths
7. **Error States**: Always test error handling and edge cases
8. **Async Testing**: Use `waitFor` for async operations
9. **User Interactions**: Use `@testing-library/user-event` for simulating user actions

### Test Layer Responsibilities

#### Unit Tests (`components/__tests__/`)
- Test individual component behavior
- Mock all external dependencies
- Focus on component-specific logic
- Test props, state, and event handling

#### Integration Tests (`routes/__tests__/`)
- Test multiple components working together
- Mock only external dependencies
- Test data flow through component hierarchy
- Test user interactions across components

#### Contract Tests
- Verify component interfaces
- Test that components receive expected props
- Validate hook call signatures
- Ensure consistent API usage

### When to Use Each Type

**Use Unit Tests when:**
- Testing complex component logic
- Testing individual hook behavior
- Testing utility functions
- Testing error scenarios specific to one component

**Use Integration Tests when:**
- Testing route components
- Testing feature workflows
- Testing component communication
- Testing state management across components

**Use Contract Tests when:**
- Verifying component interfaces don't break
- Testing that data flows correctly
- Ensuring consistent prop shapes
- Validating hook usage patterns

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

## Integration Testing Best Practices

### Route Testing Strategy

```typescript
// Example: Strategic integration test for route component
describe('TaskDetail Route - Integration Tests', () => {
  beforeEach(() => {
    // Mock external dependencies only
    vi.mocked(useTask).mockReturnValue({
      data: createMockTaskWithIterations(),
      isLoading: false,
      error: null
    });
    
    vi.mocked(useTaskSSEConnection).mockReturnValue({
      isStreaming: false
    });
  });

  it('displays task data through real component hierarchy', () => {
    const taskData = createMockTaskWithIterations({
      title: 'Integration Test Task'
    }, 2);
    
    vi.mocked(useTask).mockReturnValue({ data: taskData });
    render(<TaskDetail />);

    // Test real data flow
    expect(screen.getByText('Integration Test Task')).toBeInTheDocument();
    expect(screen.getByText(taskData.iterations[0].initial_message)).toBeInTheDocument();
  });

  it('handles user interactions across components', async () => {
    render(<TaskDetail />);
    
    // Test real component interactions
    const diffTab = screen.getByRole('tab', { name: /diff/i });
    await user.click(diffTab);
    
    // Verify state changes propagate
    expect(mockSetRightPanelView).toHaveBeenCalledWith('diff');
  });
});
```

### Debugging Integration Issues

#### Common Problems and Solutions

1. **"Component requires Provider" errors**
   ```typescript
   // Add required providers to test setup
   const wrapper = ({ children }: any) => (
     <QueryClientProvider client={queryClient}>
       <SidebarProvider>
         {children}
       </SidebarProvider>
     </QueryClientProvider>
   );
   ```

2. **"Cannot read property of undefined" errors**
   ```typescript
   // Ensure mock data has all required properties
   const completeTaskData = createMockTask({
     // Include all properties components expect
     total_cost_usd: 0.0123, // Don't let this be undefined
     num_turns: 5,
     status: 'active'
   });
   ```

3. **Tests are too slow**
   ```typescript
   // Mock the heaviest dependencies
   vi.mock('@/features/tasks/components/complex-chart', () => ({
     ComplexChart: () => <div data-testid="chart">Chart Placeholder</div>
   }));
   ```

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
it('renders large datasets efficiently', () => {
  const taskWithManyIterations = createMockTaskWithIterations({}, 100);
  
  const startTime = performance.now();
  render(<TaskDetail />);
  const endTime = performance.now();
  
  // Should render quickly even with large datasets
  expect(endTime - startTime).toBeLessThan(1000);
  expect(screen.getByText('Task')).toBeInTheDocument();
});
```

### 5. Contract Testing

```typescript
// Verify component interfaces don't break
describe('Component Contracts', () => {
  it('calls hooks with correct parameters', () => {
    render(<TaskDetail />);
    
    expect(useTask).toHaveBeenCalledWith('task-123');
    expect(useTaskSSEConnection).toHaveBeenCalledWith({ taskId: 'task-123' });
  });
  
  it('provides correct props to child components', () => {
    const mockTaskViewer = vi.fn(() => <div>TaskViewer</div>);
    
    // Don't mock - test the real prop passing
    render(<TaskDetail />);
    
    // Verify TaskViewer receives the data it needs
    expect(screen.getByText(taskData.iterations[0].initial_message)).toBeInTheDocument();
  });
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

#### Integration Testing Issues
1. **Provider Missing**: Add required providers to test wrapper
2. **Mock Data Incomplete**: Ensure all required properties are present
3. **Over-Mocking**: Reduce mocks to enable real integration testing
4. **Under-Mocking**: Mock external dependencies to prevent network calls

#### General Testing Issues
1. **Act Warnings**: Wrap state updates in `act()`
2. **Async Issues**: Always use `waitFor` for async operations
3. **Memory Leaks**: Clean up subscriptions and timers
4. **Flaky Tests**: Avoid arbitrary timeouts, use `waitFor` instead

### Migration from Over-Mocked Tests

If you have existing tests with too much mocking:

1. **Start with Route Tests**: Focus on route components first
2. **Remove Feature Component Mocks**: One at a time, remove mocks for your own components
3. **Add Required Dependencies**: Mock only what breaks without mocking
4. **Fix Data Flow Issues**: Update test data to match real component expectations
5. **Verify Improved Coverage**: Ensure you're catching more integration bugs

```typescript
// Before: Over-mocked
vi.mock('@/features/tasks/components/task-viewer');
vi.mock('@/features/tasks/components/task-chat-input');

// After: Strategic mocking
vi.mock('@/features/tasks/hooks/use-tasks'); // External dependency
// Let TaskViewer and TaskChatInput render for real!
```