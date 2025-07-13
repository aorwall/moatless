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
├── stores/
│   ├── __tests__/
│   │   └── task-ui-store.test.ts
│   └── task-ui-store.ts
├── contexts/
│   ├── __tests__/
│   │   └── task-context.test.tsx
│   └── task-context.tsx
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

### Test Organization Standards

#### File Structure
Maintain consistent test organization across all features:

```
feature/
├── __tests__/
│   ├── feature-integration.test.tsx    # Route/page integration tests
│   └── feature-data-fetching.test.tsx  # Data flow tests
├── components/
│   ├── __tests__/
│   │   ├── main-component.test.tsx     # Primary components
│   │   └── sub-component.test.tsx      # Child components
│   └── sub-components/
│       └── __tests__/
│           └── specific-component.test.tsx
├── hooks/
│   └── __tests__/
│       └── use-feature.test.ts         # Custom hooks
├── utils/
│   └── __tests__/
│       └── feature-utils.test.ts       # Utility functions
└── contexts/
    └── __tests__/
        └── feature-context.test.tsx    # Context providers
```

#### Test File Naming Conventions

Use specific naming patterns to control which tests run in different scenarios:

**Unit Tests (Fast)** - Run with `pnpm test:fast`:
- `*.test.ts` - Standard unit tests
- `*.test.tsx` - Component unit tests

**Integration Tests (Slower)** - Excluded from fast tests:
- `*.integration.test.ts` - Integration tests
- `*.integration.test.tsx` - Component integration tests
- `*connection*.test.{ts,tsx}` - Network/WebSocket tests
- `global-event-stream-provider.test.tsx` - Specific integration test

**E2E Tests (Slowest)** - Run separately:
- `*.e2e.test.ts` - End-to-end tests
- `*.e2e.test.tsx` - Full user journey tests

**Memory-Heavy Tests** - Run with increased memory:
- `markdown-preview.test.tsx` - Large DOM tests
- `preview-integration.test.tsx` - Heavy integration tests

**Test Commands**:
```bash
pnpm test:fast      # Only unit tests (fastest)
pnpm test:unit      # Same as test:fast
pnpm test:integration # Only integration tests
pnpm test:all       # All tests except E2E
pnpm test           # Standard test run (excludes some slow tests)
```

### Key Testing Principles

#### 1. Use Stable Selectors
```typescript
// ❌ BAD: Brittle text-based selection
expect(screen.getByText('Claude 3 Opus')).toBeInTheDocument()
const textarea = screen.getByPlaceholderText(/describe your coding task/i)

// ✅ GOOD: Stable data-testid attributes
expect(screen.getByTestId('model-selector')).toHaveTextContent('Claude 3 Opus')
const textarea = screen.getByTestId('message-input')
```

#### 2. Test Behavior, Not Implementation
```typescript
// ❌ BAD: Testing implementation details
expect(component.state.isLoading).toBe(true)
expect(mockFunction).toHaveBeenCalledTimes(1)

// ✅ GOOD: Testing user-facing behavior
expect(screen.getByTestId('loading-spinner')).toBeInTheDocument()
expect(screen.getByTestId('success-message')).toBeInTheDocument()
```

#### 3. Separate Business Logic from UI
```typescript
// ✅ GOOD: Extract and test logic separately
export const createTaskPayload = (
  message: string,
  repoId: string,
  model: string
): CreateTaskRequest => ({
  agentType: 'claude-code',
  message: message.trim(),
  repositoryId: repoId,
  model,
  filePaths: [], // Won't forget required fields!
})

// Test the logic independently
it('creates valid task payload', () => {
  const payload = createTaskPayload('test', 'repo', 'model')
  expect(payload.filePaths).toEqual([])
  expect(payload.agentType).toBe('claude-code')
})
```

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

### Common Testing Anti-patterns to Avoid

#### ❌ Tests That Always Pass
```typescript
// BAD: Placeholder tests that provide no value
it('should auto-scroll on first visit', () => {
  expect(true).toBe(true); // This tests nothing!
});

// GOOD: Either implement the test properly or remove it
it('should auto-scroll on first visit', () => {
  const { container } = render(<EventViewer />);
  const scrollArea = container.querySelector('[data-testid="scroll-area"]');
  
  expect(scrollArea.scrollTop).toBe(scrollArea.scrollHeight);
});
```

#### ❌ Skipped Tests Without Explanation
```typescript
// BAD: Skipped with vague reasoning
it.skip('should handle file search', () => {
  // "flaky timing issues"
});

// GOOD: Document why and include ticket number
it.skip('should handle file search - FIXME: Mock file system needed (TASK-123)', () => {
  // Test implementation pending mock setup
});
```

#### ❌ Incomplete Mock Data
```typescript
// BAD: Mock missing required fields causes runtime errors
const mockTask = {
  id: 'task-1',
  name: 'Test'
  // Missing: status, iterations, timestamps, etc.
};

// GOOD: Use complete factories with all required fields
const mockTask = createMockTaskWithIterations({
  taskId: 'task-1',
  status: 'running'
}, 2);
```

#### ❌ Inconsistent Assertions
```typescript
// BAD: Only checking element presence
it('renders task list', () => {
  render(<TaskList />);
  expect(screen.getByTestId('task-list')).toBeInTheDocument();
});

// GOOD: Test behavior and data display
it('renders task list with correct data', () => {
  const tasks = [createMockTask({ title: 'Task 1' })];
  render(<TaskList tasks={tasks} />);
  
  expect(screen.getByText('Task 1')).toBeInTheDocument();
  expect(screen.getAllByRole('listitem')).toHaveLength(1);
});
```

#### ❌ Testing Without User Context
```typescript
// BAD: Testing technical implementation
it('calls setState when clicked', () => {
  const setState = vi.fn();
  // Testing React internals
});

// GOOD: Test from user perspective
it('increments counter when increment button is clicked', async () => {
  const user = userEvent.setup();
  render(<Counter />);
  
  const button = screen.getByRole('button', { name: /increment/i });
  const counter = screen.getByTestId('counter-value');
  
  expect(counter).toHaveTextContent('0');
  
  await user.click(button);
  
  expect(counter).toHaveTextContent('1');
});
```

#### ❌ Over-Testing Third-Party Libraries
```typescript
// BAD: Testing that React Query caches data
it('caches the query result', () => {
  const { result, rerender } = renderHook(() => useQuery(...));
  rerender();
  expect(mockFetch).toHaveBeenCalledTimes(1); // Testing library behavior
});

// GOOD: Test your integration with the library
it('displays cached data on subsequent renders', () => {
  const { rerender } = render(<DataDisplay />);
  expect(screen.getByText('Loading...')).toBeInTheDocument();
  
  // After data loads
  expect(screen.getByText('Data loaded')).toBeInTheDocument();
  
  // Rerender should show data immediately (from cache)
  rerender(<DataDisplay />);
  expect(screen.getByText('Data loaded')).toBeInTheDocument();
  expect(screen.queryByText('Loading...')).not.toBeInTheDocument();
});
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

### Render Utilities
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

### Centralized Mock Utilities
```typescript
// src/test-utils/mocks/selectors.tsx
export const mockGenericSelector = () => ({
  GenericSelector: ({ value, items, testId }: any) => {
    const item = items?.find((i: any) => i.id === value)
    return (
      <div data-testid={testId}>
        {item ? <span>{item.label}</span> : null}
      </div>
    )
  },
  useRepositorySelector: vi.fn(),
  useModelSelector: vi.fn(),
  useBranchSelector: vi.fn(),
  useTemplateSelector: vi.fn(),
})

// Usage in tests
vi.mock('@/features/tasks/components/chat-input/selectors', () => 
  mockGenericSelector()
)
```

### Component Test Helpers
```typescript
// src/test-utils/component-testers/chat-input.tester.ts
export class ChatInputTester {
  private user = userEvent.setup()
  private component: RenderResult

  constructor(props: ChatInputProps) {
    this.component = renderWithProviders(<ChatInput {...props} />)
  }

  async typeMessage(message: string) {
    const input = this.component.getByTestId('message-input')
    await this.user.type(input, message)
    return this
  }

  async submit() {
    const button = this.component.getByTestId('submit-button')
    await this.user.click(button)
    return this
  }

  expectSubmitDisabled() {
    const button = this.component.getByTestId('submit-button')
    expect(button).toBeDisabled()
    return this
  }
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

### What NOT to Test

#### 1. Third-Party Library Behavior
```typescript
// ❌ BAD: Testing React Query's caching
it('caches the query result', () => {
  const { result, rerender } = renderHook(() => useTask('123'))
  rerender()
  expect(mockFetch).toHaveBeenCalledTimes(1) // Testing library behavior
})

// ✅ GOOD: Test your usage of the library
it('fetches task data on mount', () => {
  renderHook(() => useTask('123'))
  expect(mockTasksApi.getTask).toHaveBeenCalledWith('123')
})
```

#### 2. CSS Classes and Styles
```typescript
// ❌ BAD: Testing CSS classes
expect(button).toHaveClass('bg-primary hover:bg-primary-dark')

// ✅ GOOD: Test visual states through behavior
expect(button).toBeDisabled()
expect(button).toHaveAttribute('aria-pressed', 'true')
```

#### 3. Implementation Details
```typescript
// ❌ BAD: Testing internal state
expect(component.instance().state.count).toBe(5)

// ✅ GOOD: Test observable output
expect(screen.getByTestId('counter')).toHaveTextContent('5')
```

#### 4. Simple Prop Passing
```typescript
// ❌ BAD: Testing that props are passed through
it('passes className to div', () => {
  render(<Component className="test-class" />)
  expect(container.firstChild).toHaveClass('test-class')
})

// ✅ GOOD: Test meaningful behavior changes
it('disables form submission when disabled', () => {
  render(<Form disabled />)
  expect(screen.getByTestId('submit-button')).toBeDisabled()
})
```

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

### Minimum Coverage by Type
- **Components**: 80% coverage (critical paths: 90%)
- **Hooks**: 90% coverage
- **Utilities**: 95% coverage  
- **API functions**: 85% coverage
- **Context Providers**: 85% coverage
- **Route Components**: 75% coverage (focus on integration)

### Critical Path Definition
Critical paths include:
- User authentication flows
- Data submission (create/update operations)
- Real-time features (SSE, WebSocket connections)
- Error recovery mechanisms
- Core business logic

### Coverage Exceptions
Acceptable reasons for lower coverage:
- Third-party library wrappers
- Development-only code
- Deprecated code pending removal
- Generated code

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

## Test Maintenance

### Test Quality Standards

#### When to Write Tests
Write tests for:
- **Business Logic**: Any code that implements business rules or calculations
- **User Interactions**: Components that handle user input or actions
- **Data Transformations**: Functions that process or transform data
- **Error Handling**: Code that handles error cases or edge conditions
- **Critical Paths**: Features essential to application functionality

#### When NOT to Write Tests
Skip tests for:
- **Generated Code**: Files like `routeTree.gen.ts`
- **Type Definitions**: Pure TypeScript interfaces/types
- **Constants**: Simple constant exports
- **Third-Party Wrappers**: Thin wrappers around well-tested libraries
- **Styles**: CSS classes or style objects

### Test Organization Best Practices

1. **Group Related Tests**
   ```typescript
   describe('TaskViewer', () => {
     describe('Rendering', () => {
       it('displays task title', () => {});
       it('shows iteration count', () => {});
     });
     
     describe('Interactions', () => {
       it('handles iteration selection', () => {});
       it('expands/collapses sections', () => {});
     });
     
     describe('Error Handling', () => {
       it('shows error message on failure', () => {});
       it('provides retry option', () => {});
     });
   });
   ```

2. **Consistent Naming**
   - Use descriptive test names
   - Follow "should [do something] when [condition]" pattern
   - Group by feature/behavior, not implementation

## Common Testing Pitfalls to Avoid

### 1. Brittle Text Matching
```typescript
// ❌ BAD: Will break when text changes
expect(screen.getByText('Claude 3 Opus')).toBeInTheDocument()

// ✅ GOOD: Use test IDs or query by role
expect(screen.getByTestId('model-selector')).toHaveTextContent('Claude 3 Opus')
expect(screen.getByRole('combobox', { name: /model/i })).toBeInTheDocument()
```

### 2. Testing Mock Behavior
```typescript
// ❌ BAD: Testing that your mock works
const mockFn = vi.fn()
mockFn('test')
expect(mockFn).toHaveBeenCalledWith('test') // Pointless!

// ✅ GOOD: Test real component behavior
render(<Component onSubmit={mockFn} />)
await user.click(screen.getByTestId('submit'))
expect(mockFn).toHaveBeenCalledWith(expectedData)
```

### 3. Incomplete Mock Data
```typescript
// ❌ BAD: Missing required fields causes runtime errors
const mockTask = { id: '1', name: 'Test' }

// ✅ GOOD: Use complete factories
const mockTask = createMockTask({ name: 'Test' })
```

### 4. Over-Mocking
```typescript
// ❌ BAD: Mocking everything = testing nothing
vi.mock('@/features/tasks/components/task-viewer')
vi.mock('@/features/tasks/components/task-header')
vi.mock('@/features/tasks/components/event-viewer')

// ✅ GOOD: Mock only external dependencies
vi.mock('@/features/tasks/hooks/use-tasks') // API calls
// Let components render for real integration testing
```

### 5. Ignoring Async Behavior
```typescript
// ❌ BAD: Not waiting for async updates
fireEvent.click(button)
expect(screen.getByText('Success')).toBeInTheDocument() // Might fail!

// ✅ GOOD: Use waitFor
fireEvent.click(button)
await waitFor(() => {
  expect(screen.getByText('Success')).toBeInTheDocument()
})
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

### 5. Component Test Pattern

Organize component tests consistently:

```typescript
describe('ComponentName', () => {
  describe('Rendering', () => {
    it('renders with default props', () => {});
    it('renders with custom props', () => {});
    it('handles missing data gracefully', () => {});
  });
  
  describe('User Interactions', () => {
    it('handles click events', () => {});
    it('processes form submissions', () => {});
    it('responds to keyboard navigation', () => {});
  });
  
  describe('Props & State', () => {
    it('updates when props change', () => {});
    it('maintains internal state correctly', () => {});
  });
  
  describe('Error Handling', () => {
    it('displays error message on API failure', () => {});
    it('provides retry mechanism', () => {});
  });
  
  describe('Accessibility', () => {
    it('has proper ARIA labels', () => {});
    it('supports keyboard navigation', () => {});
  });
});
```

### 6. Hook Test Pattern

Structure hook tests for clarity:

```typescript
describe('useHookName', () => {
  describe('Initial State', () => {
    it('returns default values', () => {});
  });
  
  describe('State Updates', () => {
    it('updates state when action is called', () => {});
    it('handles concurrent updates', () => {});
  });
  
  describe('Side Effects', () => {
    it('fetches data on mount', () => {});
    it('cleans up on unmount', () => {});
  });
  
  describe('Error Scenarios', () => {
    it('handles network errors', () => {});
    it('provides error recovery', () => {});
  });
});
```

### 7. Contract Testing

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

### 8. Context Provider Testing

Test context providers properly:

```typescript
import { renderHook } from '@testing-library/react';
import { TaskDataProvider, useTaskData } from '../contexts/task-data-provider';

describe('TaskDataProvider', () => {
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <TaskDataProvider taskId="test-123">
      {children}
    </TaskDataProvider>
  );

  it('provides task data to children', () => {
    const { result } = renderHook(() => useTaskData(), { wrapper });
    
    expect(result.current.taskId).toBe('test-123');
    expect(result.current.task).toBeDefined();
  });

  it('handles loading states', () => {
    vi.mocked(useTask).mockReturnValue({
      isLoading: true,
      data: undefined,
      error: null
    });

    const { result } = renderHook(() => useTaskData(), { wrapper });
    expect(result.current.isLoading).toBe(true);
  });

  it('propagates errors correctly', () => {
    const error = new Error('Failed to load');
    vi.mocked(useTask).mockReturnValue({
      isLoading: false,
      data: undefined,
      error
    });

    const { result } = renderHook(() => useTaskData(), { wrapper });
    expect(result.current.error).toBe(error);
  });
});
```

### 9. React Context and Reducer Testing

Test React Context providers and reducers for state management:

```typescript
// Example from preview-reducer.test.tsx
describe('PreviewReducer', () => {
  it('handles rapid state changes without race conditions', () => {
    let state = initialState;
    
    // Simulate rapid updates
    state = previewReducer(state, { type: 'SET_LOADING', payload: true });
    state = previewReducer(state, { type: 'SET_CONTENT', payload: 'content1' });
    state = previewReducer(state, { type: 'SET_LOADING', payload: false });
    
    expect(state.isLoading).toBe(false);
    expect(state.content).toBe('content1');
  });

  it('maintains state immutability', () => {
    const originalState = { ...initialState };
    const newState = previewReducer(originalState, {
      type: 'SET_CONFIG',
      payload: { theme: 'dark' }
    });
    
    expect(newState).not.toBe(originalState);
    expect(originalState.config).toEqual({}); // Original unchanged
  });

  it('provides state isolation between contexts', () => {
    const { result: context1 } = renderHook(() => usePreview(), {
      wrapper: ({ children }) => <PreviewProvider>{children}</PreviewProvider>
    });
    
    const { result: context2 } = renderHook(() => usePreview(), {
      wrapper: ({ children }) => <PreviewProvider>{children}</PreviewProvider>
    });
    
    act(() => {
      context1.current.setContent('content1');
    });
    
    expect(context1.current.content).toBe('content1');
    expect(context2.current.content).toBe(''); // Isolated
  });
});
```

### 10. Performance and Memory Leak Testing

Test for performance regressions and memory leaks:

```typescript
// Example from infinite-loop-detection.test.tsx
describe('Performance and Memory Tests', () => {
  it('prevents infinite render loops', () => {
    const renderCount = vi.fn();
    
    const ProblematicComponent = () => {
      renderCount();
      const [state, setState] = useState(0);
      
      // This would cause infinite loop if not handled properly
      useEffect(() => {
        setState(prev => prev + 1);
      }); // Missing dependency array
      
      return <div>{state}</div>;
    };
    
    const { unmount } = render(<ProblematicComponent />);
    
    // Should not render infinitely
    expect(renderCount).toHaveBeenCalledTimes(1);
    unmount();
  });

  it('cleans up subscriptions to prevent memory leaks', () => {
    const unsubscribe = vi.fn();
    const subscribe = vi.fn(() => unsubscribe);
    
    const { unmount } = renderHook(() => useSubscription(subscribe));
    
    unmount();
    
    expect(unsubscribe).toHaveBeenCalled();
  });

  it('handles large datasets efficiently', () => {
    const startTime = performance.now();
    const largeData = Array.from({ length: 10000 }, (_, i) => ({
      id: i,
      name: `Item ${i}`
    }));
    
    render(<VirtualizedList items={largeData} />);
    
    const renderTime = performance.now() - startTime;
    expect(renderTime).toBeLessThan(100); // Should render quickly
  });
});
```

### 11. SSE/WebSocket Testing

Test real-time features and event streams:

```typescript
describe('SSE Connection Testing', () => {
  let mockEventSource: MockEventSource;
  
  beforeEach(() => {
    mockEventSource = new MockEventSource();
    global.EventSource = vi.fn(() => mockEventSource);
  });

  it('handles connection lifecycle', async () => {
    const { result } = renderHook(() => useTaskSSEConnection('task-123'));
    
    expect(result.current.isConnected).toBe(false);
    
    // Simulate connection
    act(() => {
      mockEventSource.triggerOpen();
    });
    
    expect(result.current.isConnected).toBe(true);
    
    // Simulate disconnection
    act(() => {
      mockEventSource.triggerError(new Event('error'));
    });
    
    expect(result.current.isConnected).toBe(false);
  });

  it('processes streaming events', async () => {
    const onMessage = vi.fn();
    renderHook(() => useTaskSSEConnection('task-123', { onMessage }));
    
    const event = new MessageEvent('message', {
      data: JSON.stringify({ type: 'update', payload: { status: 'completed' } })
    });
    
    act(() => {
      mockEventSource.triggerMessage(event);
    });
    
    expect(onMessage).toHaveBeenCalledWith({
      type: 'update',
      payload: { status: 'completed' }
    });
  });

  it('handles reconnection with backoff', async () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useTaskSSEConnection('task-123'));
    
    // First connection fails
    act(() => {
      mockEventSource.triggerError(new Event('error'));
    });
    
    expect(result.current.reconnectAttempts).toBe(1);
    
    // Wait for backoff
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    
    expect(EventSource).toHaveBeenCalledTimes(2); // Initial + retry
    vi.useRealTimers();
  });
});
```

### 12. Form Testing Strategies

Test complex forms with validation and third-party components:

```typescript
describe('Form Testing', () => {
  it('validates form fields', async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    
    render(<RepositoryForm onSubmit={onSubmit} />);
    
    // Submit without filling required fields
    await user.click(screen.getByTestId('submit-button'));
    
    expect(screen.getByText('Repository name is required')).toBeInTheDocument();
    expect(onSubmit).not.toHaveBeenCalled();
    
    // Fill required fields
    await user.type(screen.getByTestId('repo-name-input'), 'my-repo');
    await user.type(screen.getByTestId('repo-url-input'), 'https://github.com/user/repo');
    
    await user.click(screen.getByTestId('submit-button'));
    
    expect(onSubmit).toHaveBeenCalledWith({
      name: 'my-repo',
      url: 'https://github.com/user/repo'
    });
  });

  it('handles complex UI components gracefully', async () => {
    const user = userEvent.setup();
    render(<FormWithSelect />);
    
    // For hard-to-test components, focus on the outcome
    const selectTrigger = screen.getByTestId('branch-selector');
    await user.click(selectTrigger);
    
    // Instead of testing the dropdown internals, test the result
    // This might involve mocking the select component
    expect(screen.getByTestId('selected-branch')).toHaveTextContent('main');
  });

  it('handles async validation', async () => {
    const checkUsername = vi.fn().mockImplementation(async (username) => {
      await new Promise(resolve => setTimeout(resolve, 100));
      return username === 'taken';
    });
    
    render(<UsernameForm checkUsername={checkUsername} />);
    
    await userEvent.type(screen.getByTestId('username-input'), 'taken');
    
    // Wait for async validation
    await waitFor(() => {
      expect(screen.getByText('Username is already taken')).toBeInTheDocument();
    });
  });
});
```

### 13. Accessibility Testing

Test for accessibility compliance:

```typescript
describe('Accessibility Tests', () => {
  it('has proper ARIA labels', () => {
    render(<TaskList tasks={mockTasks} />);
    
    expect(screen.getByRole('list', { name: /tasks/i })).toBeInTheDocument();
    expect(screen.getAllByRole('listitem')).toHaveLength(mockTasks.length);
    
    // Check for proper heading hierarchy
    const headings = screen.getAllByRole('heading');
    expect(headings[0]).toHaveProperty('tagName', 'H1');
  });

  it('supports keyboard navigation', async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    
    render(<TaskList tasks={mockTasks} onSelect={onSelect} />);
    
    // Tab to first item
    await user.tab();
    expect(screen.getAllByRole('listitem')[0]).toHaveFocus();
    
    // Navigate with arrow keys
    await user.keyboard('[ArrowDown]');
    expect(screen.getAllByRole('listitem')[1]).toHaveFocus();
    
    // Select with Enter
    await user.keyboard('[Enter]');
    expect(onSelect).toHaveBeenCalledWith(mockTasks[1]);
  });

  it('announces dynamic content changes', async () => {
    render(<NotificationArea />);
    
    act(() => {
      // Trigger a notification
      showNotification('Task completed successfully');
    });
    
    // Check for live region
    const liveRegion = screen.getByRole('status');
    expect(liveRegion).toHaveTextContent('Task completed successfully');
    expect(liveRegion).toHaveAttribute('aria-live', 'polite');
  });

  it('has sufficient color contrast', () => {
    // This would typically be done with axe-core
    // Example with jest-axe:
    const { container } = render(<Button>Click me</Button>);
    // await expect(container).toHaveNoAccessibilityViolations();
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

### Test Naming Best Practices

Use descriptive test names that explain the expected behavior:

```typescript
// ❌ BAD: Vague test names
it('renders correctly');
it('handles click');
it('works with props');

// ✅ GOOD: Descriptive behavior-focused names
it('displays task title and status when task data is provided');
it('disables submit button when form has validation errors');
it('scrolls to bottom when new message is received');
it('retries connection with exponential backoff after network failure');
```

### Test Data Management

#### Use Factory Functions
```typescript
// src/test-utils/factories/task.factory.ts
export const createMockTask = (overrides?: Partial<Task>): Task => ({
  id: 'task-' + Math.random().toString(36).substr(2, 9),
  title: 'Default Task Title',
  status: 'active',
  created_at: new Date().toISOString(),
  repository_id: 'repo-123',
  agent_type: 'claude-code',
  ...overrides,
});

export const createMockTaskWithIterations = (
  taskOverrides?: Partial<Task>,
  iterationCount = 1
): Task => {
  const task = createMockTask(taskOverrides);
  const iterations = Array.from({ length: iterationCount }, (_, i) => 
    createMockIteration({
      task_id: task.id,
      iteration_number: i + 1,
    })
  );
  
  return { ...task, iterations };
};
```

#### Avoid Hardcoded Test Data
```typescript
// ❌ BAD: Hardcoded data scattered across tests
const task = {
  id: '123',
  title: 'Test',
  // Missing fields...
};

// ✅ GOOD: Use factories with meaningful overrides
const task = createMockTask({
  title: 'Implement user authentication',
  status: 'completed',
});
```