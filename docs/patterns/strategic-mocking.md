# Strategic Mocking Pattern

## Problem

Testing modern React applications with complex integrations poses challenges:
- Over-mocking leads to tests that pass but don't catch real bugs
- Under-mocking makes tests slow, flaky, and dependent on external services
- Inconsistent mocking strategies across the codebase
- Tests don't reflect actual user interactions
- Integration bugs slip through despite good unit test coverage

## Solution

Strategic Mocking balances isolation and integration by mocking only what's necessary:
- **Mock External Dependencies**: APIs, third-party services, browser APIs
- **Mock Complex UI Libraries**: That don't affect business logic
- **Don't Mock Business Logic**: Test real component interactions
- **Don't Mock Simple Components**: Basic UI elements should be real
- **Smart Contract Testing**: Verify component interfaces work together

## Implementation

### Type Safety for Mocks

- SHOULD model partial dependencies with `Partial<T>` or `Pick<T, K>` plus `as const` casts instead of sprinkling `any`.
- MAY use `any` in tests only with an inline lint suppression that documents why the unsafe cast is acceptable.
- MUST prefer `unknown` when capturing arbitrary JSON payloads and narrow explicitly inside the test.

```typescript
// CORRECT – narrow unknown payload and justify lone any usage
const payload: unknown = await readFixture('task-created.json')

if (!isTaskCreatedEvent(payload)) {
  throw new Error('Invalid fixture: expected task created event')
}

// This factory intentionally returns a sparse object for readability
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const sparseTaskService = { create: vi.fn().mockResolvedValue(payload.task) } as any

// WRONG – broad any hides missing fields and bypasses type checks
const mockService: any = { create: vi.fn().mockResolvedValue({}) }
```

### Mocking Strategy Framework

```typescript
// Mocking decision matrix
enum MockingStrategy {
  ALWAYS_MOCK = 'always_mock',      // External dependencies
  NEVER_MOCK = 'never_mock',        // Business logic components
  CONDITIONAL_MOCK = 'conditional', // Complex UI libraries
  CONTRACT_TEST = 'contract_test',  // Interface verification
}

interface MockingRule {
  pattern: string | RegExp;
  strategy: MockingStrategy;
  reason: string;
  mockImplementation?: () => any;
}

const MOCKING_RULES: MockingRule[] = [
  // Always mock external dependencies
  {
    pattern: /^@\/lib\/api\//,
    strategy: MockingStrategy.ALWAYS_MOCK,
    reason: 'External API calls should be mocked for test isolation'
  },
  
  // Always mock SSE connections
  {
    pattern: /use.*sse.*connection/i,
    strategy: MockingStrategy.ALWAYS_MOCK,
    reason: 'Server-sent events need controlled test environment'
  },
  
  // Mock complex UI libraries
  {
    pattern: /@\/lib\/components\/ui\/resizable/,
    strategy: MockingStrategy.CONDITIONAL_MOCK,
    reason: 'Complex layout library - mock for test simplicity'
  },
  
  // Never mock feature components
  {
    pattern: /^@\/features\/.*\/components/,
    strategy: MockingStrategy.NEVER_MOCK,
    reason: 'Feature features should be tested with real integrations'
  },
  
  // Contract test for contexts
  {
    pattern: /.*Context$/,
    strategy: MockingStrategy.CONTRACT_TEST,
    reason: 'Context providers need interface verification'
  },
];
```

### Strategic Mock Setup

```typescript
// Mock configuration for different test types
export function setupStrategicMocks(testType: 'unit' | 'integration' | 'e2e') {
  switch (testType) {
    case 'unit':
      setupUnitTestMocks();
      break;
    case 'integration':
      setupIntegrationTestMocks();
      break;
    case 'e2e':
      setupE2ETestMocks();
      break;
  }
}

function setupUnitTestMocks() {
  // Mock all external dependencies
  vi.mock('@/lib/api/client', () => ({
    apiClient: {
      get: vi.fn(),
      post: vi.fn(),
      put: vi.fn(),
      delete: vi.fn(),
    }
  }));
  
  // Mock complex UI libraries
  vi.mock('@/lib/features/ui/resizable', () => ({
    ResizablePanelGroup: ({ children }: { children: React.ReactNode }) => 
      <div data-testid="resizable-panel-group">{children}</div>,
    ResizablePanel: ({ children }: { children: React.ReactNode }) => 
      <div data-testid="resizable-panel">{children}</div>,
    ResizableHandle: () => <div data-testid="resizable-handle" />,
  }));
  
  // Mock browser APIs
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: vi.fn().mockImplementation(query => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
}

function setupIntegrationTestMocks() {
  // Mock only external services, keep internal integrations
  vi.mock('@/lib/api/client', () => ({
    apiClient: createMockApiClient()
  }));
  
  // Mock SSE connections but keep event processing
  vi.mock('@/features/events/hooks/use-global-event-stream', () => ({
    useGlobalEventStream: () => ({
      subscribe: vi.fn((key, callback) => {
        // Return unsubscribe function
        return () => {};
      }),
      isConnected: true,
    }),
  }));
  
  // Keep most UI components real for integration testing
  // Only mock the most complex ones
  vi.mock('@/lib/features/ui/resizable', () => ({
    ResizablePanelGroup: ({ children }: { children: React.ReactNode }) => 
      <div data-testid="resizable-panel-group">{children}</div>,
    ResizablePanel: ({ children }: { children: React.ReactNode }) => 
      <div data-testid="resizable-panel">{children}</div>,
    ResizableHandle: () => <div data-testid="resizable-handle" />,
  }));
}
```

### Mock Factory Pattern

```typescript
// Create consistent mock implementations
export function createMockTask(overrides?: Partial<Task>): Task {
  return {
    id: 'task-1',
    title: 'Test Task',
    description: 'Test task description',
    status: 'open',
    repositoryId: 'repo-1',
    createdAt: '2024-01-01T00:00:00Z',
    updatedAt: '2024-01-01T00:00:00Z',
    iterations: [],
    ...overrides,
  };
}

export function createMockTaskWithIterations(iterationCount = 2): Task {
  const iterations = Array.from({ length: iterationCount }, (_, i) => ({
    id: `iter-${i + 1}`,
    taskId: 'task-1',
    message: `Test message ${i + 1}`,
    status: i === iterationCount - 1 ? 'active' : 'completed',
    createdAt: new Date(Date.now() - (iterationCount - i) * 60000).toISOString(),
  }));
  
  return createMockTask({ iterations });
}

export function createMockApiClient() {
  return {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    delete: vi.fn(),
  };
}

// Smart mock that provides realistic responses
export function createSmartMockApiClient() {
  const mockData = {
    tasks: [createMockTask(), createMockTask({ id: 'task-2', title: 'Task 2' })],
    repositories: [
      { id: 'repo-1', name: 'Test Repo', path: '/test/path' },
    ],
  };
  
  return {
    get: vi.fn().mockImplementation((url: string) => {
      if (url.includes('/tasks')) {
        if (url.includes('/tasks/')) {
          const taskId = url.split('/tasks/')[1].split('/')[0];
          return Promise.resolve(mockData.tasks.find(t => t.id === taskId));
        }
        return Promise.resolve(mockData.tasks);
      }
      
      if (url.includes('/repositories')) {
        return Promise.resolve(mockData.repositories);
      }
      
      return Promise.reject(new Error(`Unmocked URL: ${url}`));
    }),
    
    post: vi.fn().mockImplementation((url: string, data: any) => {
      if (url.includes('/tasks')) {
        const newTask = createMockTask({ ...data, id: `task-${Date.now()}` });
        mockData.tasks.push(newTask);
        return Promise.resolve(newTask);
      }
      
      return Promise.resolve({ id: 'created', ...data });
    }),
    
    put: vi.fn().mockResolvedValue({ success: true }),
    delete: vi.fn().mockResolvedValue({ success: true }),
  };
}
```

### Contract Testing Pattern

```typescript
// Test component interfaces without mocking internals
describe('TaskProvider Contract', () => {
  it('should provide expected context values', () => {
    const TestConsumer = () => {
      const context = useTaskContext();
      
      // Verify the contract without mocking the provider
      expect(context).toMatchObject({
        task: expect.any(Object),
        isLoading: expect.any(Boolean),
        error: expect.any(Object) || null,
        refresh: expect.any(Function),
      });
      
      return <div>Test</div>;
    };
    
    render(
      <TaskProvider taskId="task-1">
        <TestConsumer />
      </TaskProvider>
    );
  });
  
  it('should handle task updates through context', async () => {
    const TestComponent = () => {
      const { task, updateTask } = useTaskContext();
      
      return (
        <div>
          <span data-testid="task-title">{task?.title}</span>
          <button
            onClick={() => updateTask({ title: 'Updated Title' })}
            data-testid="update-button"
          >
            Update
          </button>
        </div>
      );
    };
    
    // Use real provider with mock API
    const mockApi = createSmartMockApiClient();
    vi.mocked(apiClient.put).mockImplementation(mockApi.put);
    
    render(
      <TaskProvider taskId="task-1">
        <TestComponent />
      </TaskProvider>
    );
    
    await waitFor(() => {
      expect(screen.getByTestId('task-title')).toHaveTextContent('Test Task');
    });
    
    fireEvent.click(screen.getByTestId('update-button'));
    
    await waitFor(() => {
      expect(mockApi.put).toHaveBeenCalledWith(
        expect.stringContaining('/tasks/task-1'),
        expect.objectContaining({ title: 'Updated Title' })
      );
    });
  });
});
```

### Integration Test Pattern

```typescript
// Test real component integration with minimal mocking
describe('TaskDetail Integration', () => {
  beforeEach(() => {
    // Only mock external dependencies
    vi.mocked(useTaskSSEConnection).mockReturnValue({
      events: [],
      isConnected: true,
      error: null,
    });
    
    vi.mocked(useTask).mockReturnValue({
      data: createMockTaskWithIterations(),
      isLoading: false,
      error: null,
    });
  });
  
  it('should display task data through component hierarchy', async () => {
    // Don't mock TaskViewer, TaskHeader, or TaskIteration components
    // Test the real integration
    render(<TaskDetail taskId="task-1" />);
    
    // Verify data flows through real components
    expect(screen.getByText('Test Task')).toBeInTheDocument();
    expect(screen.getByText('Test message 1')).toBeInTheDocument();
    expect(screen.getByText('Test message 2')).toBeInTheDocument();
    
    // Verify component interactions work
    const iterationButtons = screen.getAllByRole('button', { name: /iteration/i });
    expect(iterationButtons).toHaveLength(2);
    
    fireEvent.click(iterationButtons[1]);
    
    // Verify state changes propagate through components
    await waitFor(() => {
      expect(screen.getByText('Test message 2')).toHaveClass('selected');
    });
  });
  
  it('should handle loading and error states across features', async () => {
    vi.mocked(useTask).mockReturnValue({
      data: null,
      isLoading: true,
      error: null,
    });
    
    render(<TaskDetail taskId="task-1" />);
    
    // Should show loading state in real components
    expect(screen.getByTestId('task-detail-skeleton')).toBeInTheDocument();
    
    // Test error state
    rerender(
      vi.mocked(useTask).mockReturnValue({
        data: null,
        isLoading: false,
        error: new Error('Failed to load task'),
      })
    );
    
    // Should show error in real error boundary
    expect(screen.getByText(/failed to load task/i)).toBeInTheDocument();
  });
});
```

### Test Utilities

```typescript
// Helper functions for strategic testing
export function createTestWrapper(options: {
  withRouter?: boolean;
  withQueryClient?: boolean;
  mockLevel?: 'unit' | 'integration';
} = {}) {
  const { withRouter = true, withQueryClient = true, mockLevel = 'integration' } = options;
  
  setupStrategicMocks(mockLevel);
  
  return function TestWrapper({ children }: { children: React.ReactNode }) {
    let wrapper = <>{children}</>;
    
    if (withQueryClient) {
      const queryClient = new QueryClient({
        defaultOptions: {
          queries: { retry: false },
          mutations: { retry: false },
        },
      });
      
      wrapper = (
        <QueryClientProvider client={queryClient}>
          {wrapper}
        </QueryClientProvider>
      );
    }
    
    if (withRouter) {
      wrapper = (
        <MemoryRouter>
          {wrapper}
        </MemoryRouter>
      );
    }
    
    return wrapper;
  };
}

export function renderWithProviders(
  ui: React.ReactElement,
  options?: {
    mockLevel?: 'unit' | 'integration';
    initialEntries?: string[];
  }
) {
  const { mockLevel = 'integration', initialEntries = ['/'] } = options || {};
  
  const Wrapper = createTestWrapper({
    withRouter: true,
    withQueryClient: true,
    mockLevel,
  });
  
  return render(ui, { wrapper: Wrapper });
}

// Assertion helpers
export function expectComponentIntegration(
  parentComponent: string,
  childComponent: string,
  expectedProps: Record<string, any>
) {
  const parent = screen.getByTestId(parentComponent);
  const child = within(parent).getByTestId(childComponent);
  
  Object.entries(expectedProps).forEach(([prop, value]) => {
    expect(child).toHaveAttribute(`data-${prop}`, String(value));
  });
}
```

## Best Practices

### Do's
- **Mock external boundaries** (APIs, browser APIs, third-party services)
- **Test real business logic** component interactions
- **Use factories** for consistent test data
- **Verify contracts** between components
- **Layer your mocking strategy** by test type

### Don'ts
- Don't mock components you own and control
- Don't mock simple, stable dependencies
- Don't over-mock in integration tests
- Don't ignore mock setup in different test environments
- Don't forget to reset mocks between tests

### Mocking Decision Framework
```typescript
function shouldMock(dependency: string): boolean {
  // Always mock
  if (isExternalService(dependency)) return true;
  if (isBrowserAPI(dependency)) return true;
  if (isThirdPartyLibrary(dependency)) return true;
  
  // Sometimes mock
  if (isComplexUILibrary(dependency)) return true; // Usually yes
  if (isSlowComponent(dependency)) return true; // In unit tests
  
  // Never mock
  if (isBusinessLogicComponent(dependency)) return false;
  if (isSimpleUIComponent(dependency)) return false;
  if (isOwnedUtility(dependency)) return false;
  
  return false; // Default to real
}
```
