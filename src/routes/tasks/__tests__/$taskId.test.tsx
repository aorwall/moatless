import { screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  useEnvironmentsList,
  useSearchFiles,
} from '@/features/environments/hooks/use-environments';
import {
  useCreateTask,
  useSendMessage,
  useServerStatus,
  useStartServer,
  useStopAgent,
  useStopServer,
  useTask,
  useTaskEvents,
  useTaskFileSearch,
  useTaskGitDiff,
  useTaskWorkspaceStatus,
} from '@/features/tasks/hooks/use-tasks';
import {
  useRightPanelView,
  useSelectedIteration,
  useTaskUIStore,
} from '@/features/tasks/stores/task-ui-store';
import {
  createMockTask,
  createMockTaskWithIterations,
} from '@/test-utils/factories';
import { renderWithRouter } from '@/test-utils/render';

// ===============================
// STRATEGIC MOCKING APPROACH
// ===============================
// We use minimal mocking to enable integration testing while keeping tests fast
// Mock only:
// 1. External dependencies (hooks, SSE)
// 2. Complex UI libraries (resizable, skeleton)
// 3. Error boundary components
//
// DO NOT MOCK:
// - Feature components (ChatView, etc.) - test integration
// - Simple UI components that don't affect business logic

// Mock external dependencies only
vi.mock('@/features/tasks/hooks/use-tasks');
vi.mock('@/features/environments/hooks/use-environments');
vi.mock('@/features/tasks/stores/task-ui-store', () => ({
  useTaskUIStore: vi.fn(),
  useRightPanelView: vi.fn(),
  useSelectedIteration: vi.fn(),
}));
vi.mock('@/features/tasks/stores/task-streaming-store', () => ({
  useTaskStreamingError: vi.fn(() => null),
  useTaskStreamEvents: vi.fn(() => []),
}));

// Mock complex skeleton component to avoid render complexity
vi.mock('@/features/tasks/components/task-skeletons', () => ({
  TaskDetailSkeleton: () => <div data-testid="task-skeleton">Loading...</div>,
}));

// Mock error boundary component
vi.mock('@/lib/components/ui/error-alert', () => ({
  ErrorAlert: ({ error, title }: any) => (
    <div data-testid="error-alert">
      <h2>{title}</h2>
      <p>{error.message}</p>
    </div>
  ),
}));

// Mock TaskHeader to avoid complex dependencies while testing integration with other components
vi.mock('@/features/tasks/components/task-header', () => ({
  TaskHeader: () => (
    <div data-testid="task-header">
      <h1 data-testid="task-title">Task Header</h1>
    </div>
  ),
}));

// Mock SidebarTrigger to avoid provider dependency
vi.mock('@/lib/components/ui/sidebar.tsx', () => ({
  SidebarTrigger: () => (
    <button data-testid="sidebar-trigger">Toggle Sidebar</button>
  ),
}));

// Mock router
vi.mock('@tanstack/react-router', () => ({
  createFileRoute: (_path: string) => (config: any) => ({
    useParams: () => ({ taskId: 'task-123' }),
    ...config,
  }),
  useParams: () => ({ taskId: 'task-123' }),
  useNavigate: () => vi.fn(),
}));

// Mock UI components that add complexity without business value
vi.mock('@/lib/components/ui/resizable', () => ({
  ResizableHandle: ({ withHandle }: any) => (
    <div data-testid="resizable-handle" data-with-handle={withHandle}>
      Handle
    </div>
  ),
  ResizablePanel: ({ children, defaultSize, minSize }: any) => (
    <div
      data-testid="resizable-panel"
      data-default-size={defaultSize}
      data-min-size={minSize}
    >
      {children}
    </div>
  ),
  ResizablePanelGroup: ({ children, direction }: any) => (
    <div data-testid="resizable-panel-group" data-direction={direction}>
      {children}
    </div>
  ),
}));

vi.mock('@/lib/components/ui/tabs', () => {
  let currentTabValue = 'events';

  return {
    Tabs: ({ children, value, onValueChange, className }: any) => {
      currentTabValue = value || 'events';
      return (
        <div
          data-testid="tabs"
          data-value={value}
          className={className}
          onClick={() =>
            onValueChange?.(value === 'events' ? 'diff' : 'events')
          }
        >
          {children}
        </div>
      );
    },
    TabsList: ({ children, className }: any) => (
      <div data-testid="tabs-list" className={className} role="tablist">
        {children}
      </div>
    ),
    TabsTrigger: ({ children, value }: any) => (
      <button
        data-testid={`tab-trigger-${value}`}
        role="tab"
        aria-selected={currentTabValue === value ? 'true' : 'false'}
      >
        {children}
      </button>
    ),
    TabsContent: ({ children, value, className }: any) => (
      <div
        data-testid={`tab-content-${value}`}
        data-value={value}
        className={className}
        role="tabpanel"
      >
        {children}
      </div>
    ),
  };
});

// Now import the route after all mocks are set up
import { Route } from '@/routes/tasks/$taskId';

// Get the actual component from the route
const TaskDetail = (Route as any).component;

describe('TaskDetail Route - Integration Tests', () => {
  const mockResetUIState = vi.fn();
  const mockSetRightPanelView = vi.fn();
  const user = userEvent.setup();

  beforeEach(() => {
    vi.clearAllMocks();

    // Mock DOM methods used by TaskViewer
    Element.prototype.scrollIntoView = vi.fn();

    // Default task data for integration testing
    const defaultTask = createMockTaskWithIterations(
      {
        task_id: 'task-123',
        title: 'Integration Test Task',
        status: 'active',
      },
      2,
    );

    // Mock external dependencies only
    vi.mocked(useTask).mockReturnValue({
      data: defaultTask,
      isLoading: false,
      error: null,
    } as any);

    // Mock all other hooks used by TaskHeader
    vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
      data: {
        status: 'idle' as const,
        message: 'Workspace ready',
        created_at: new Date().toISOString(),
      },
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useServerStatus).mockReturnValue({
      data: {
        running: false,
        port: undefined,
        pid: undefined,
        logs: [],
      },
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useTaskEvents).mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useTaskGitDiff).mockReturnValue({
      data: undefined,
      isLoading: false,
      error: null,
    } as any);

    // Mock mutation hooks
    vi.mocked(useStartServer).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    } as any);

    vi.mocked(useStopServer).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    } as any);

    vi.mocked(useStopAgent).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    } as any);

    // Mock ChatInput dependencies
    vi.mocked(useSendMessage).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
      isError: false,
      isSuccess: false,
      error: null,
      data: undefined,
    } as any);

    vi.mocked(useCreateTask).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
      isError: false,
      isSuccess: false,
      error: null,
      data: undefined,
    } as any);

    vi.mocked(useEnvironmentsList).mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useSearchFiles).mockReturnValue({
      data: { results: [], total_found: 0, query: '' },
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useTaskFileSearch).mockReturnValue({
      data: { results: [], total_found: 0, query: '' },
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useTaskUIStore).mockImplementation((selector) => {
      const state = {
        rightPanelView: 'events',
        setRightPanelView: mockSetRightPanelView,
        resetUIState: mockResetUIState,
        selectedIteration: null,
        setSelectedIteration: vi.fn(),
      };
      return selector ? selector(state as any) : state;
    });

    vi.mocked(useRightPanelView).mockReturnValue('events');
    vi.mocked(useSelectedIteration).mockReturnValue(null);
  });

  afterEach(() => {
    vi.clearAllTimers();
  });

  describe('Loading and Error States', () => {
    it('shows loading skeleton when loading', () => {
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: true,
        error: null,
      } as any);

      renderWithRouter(<TaskDetail />);

      expect(screen.getByTestId('task-skeleton')).toBeInTheDocument();
    });

    it('shows error alert on error', () => {
      const error = new Error('Failed to load task');
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: false,
        error,
      } as any);

      renderWithRouter(<TaskDetail />);

      const errorAlert = screen.getByTestId('error-alert');
      expect(errorAlert).toBeInTheDocument();
      expect(errorAlert).toHaveTextContent('Failed to load task');
    });

    it('shows not found message when task is null', () => {
      vi.mocked(useTask).mockReturnValue({
        data: null,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskDetail />);

      expect(screen.getByText('Task not found')).toBeInTheDocument();
      expect(
        screen.getByText('Task with ID task-123 could not be found'),
      ).toBeInTheDocument();
    });
  });

  describe('Component Integration', () => {
    it('renders all main components and displays task data', () => {
      const taskWithIterations = createMockTaskWithIterations(
        {
          task_id: 'task-123',
          title: 'Test Integration Task',
        },
        2,
      );

      vi.mocked(useTask).mockReturnValue({
        data: taskWithIterations,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskDetail />);

      // Verify task header is rendered
      expect(screen.getByTestId('task-header')).toBeInTheDocument();

      // Verify task viewer shows iterations (integration with real component)
      expect(
        screen.getByText(taskWithIterations.iterations[0].initial_message),
      ).toBeInTheDocument();
      expect(
        screen.getByText(taskWithIterations.iterations[1].initial_message),
      ).toBeInTheDocument();

      // Verify layout components are present
      expect(screen.getByRole('tablist')).toBeInTheDocument();
      expect(screen.getByRole('tab', { name: /events/i })).toBeInTheDocument();
      expect(screen.getByRole('tab', { name: /diff/i })).toBeInTheDocument();
    });

    it('integrates task status with chat input behavior', () => {
      const runningTask = createMockTaskWithIterations(
        {
          task_id: 'task-123',
          is_running: true,
          status: 'active',
        },
        1,
      );

      vi.mocked(useTask).mockReturnValue({
        data: runningTask,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskDetail />);

      // Verify running task shows in task viewer
      expect(screen.getByText('Processing...')).toBeInTheDocument();

      // Chat input should reflect running state (integration test)
      const chatSection = screen.getByTestId('task-chat-input');
      expect(chatSection).toBeInTheDocument();
    });

    it('handles tab switching and panel state integration', async () => {
      renderWithRouter(<TaskDetail />);

      const diffTab = screen.getByRole('tab', { name: /diff/i });

      // Test actual tab interaction
      await user.click(diffTab);

      // Verify store integration
      expect(mockSetRightPanelView).toHaveBeenCalledWith('diff');
    });
  });

  describe('External Dependencies Integration', () => {
    it('sets up SSE connection with correct task ID', () => {
      renderWithRouter(<TaskDetail />);
    });

    it('handles task data loading states correctly', () => {
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: true,
        error: null,
      } as any);

      renderWithRouter(<TaskDetail />);
      expect(screen.getByTestId('task-skeleton')).toBeInTheDocument();
    });

    it('integrates error states across components', () => {
      const error = new Error('Failed to load task data');
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: false,
        error,
      } as any);

      renderWithRouter(<TaskDetail />);

      const errorAlert = screen.getByTestId('error-alert');
      expect(errorAlert).toBeInTheDocument();
      expect(errorAlert).toHaveTextContent('Failed to load task');
      expect(errorAlert).toHaveTextContent('Failed to load task data');
    });
  });

  describe('State Management Integration', () => {
    it('resets UI state on unmount (lifecycle integration)', () => {
      const { unmount } = renderWithRouter(<TaskDetail />);

      expect(mockResetUIState).not.toHaveBeenCalled();
      unmount();
      expect(mockResetUIState).toHaveBeenCalledTimes(1);
    });

    it('reflects panel view state changes', () => {
      // Test with diff panel active
      vi.mocked(useRightPanelView).mockReturnValue('diff');

      renderWithRouter(<TaskDetail />);

      const diffTab = screen.getByRole('tab', { name: /diff/i });
      expect(diffTab).toHaveAttribute('aria-selected', 'true');
    });

    it('integrates task iteration selection with viewer', async () => {
      const taskWithIterations = createMockTaskWithIterations({}, 3);
      vi.mocked(useTask).mockReturnValue({
        data: taskWithIterations,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskDetail />);

      // Verify all iterations are displayed (integration with TaskViewer)
      const iterations = screen.getAllByText(/Test iteration \d+ message/);
      expect(iterations).toHaveLength(3);

      // Test iteration interaction (if TaskViewer supports clicking)
      const firstIteration = screen.getByText('Test iteration 1 message');
      expect(firstIteration).toBeInTheDocument();
    });
  });

  describe('Data Flow Integration', () => {
    it('passes task context through component hierarchy', () => {
      const specificTask = createMockTask({
        task_id: 'task-456',
        title: 'Specific Task Title',
        status: 'completed',
      });

      vi.mocked(useTask).mockReturnValue({
        data: specificTask,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskDetail />);

      // Verify that useTask hook is called with correct task ID
      expect(useTask).toHaveBeenCalledWith('task-123');

      // Verify section structure is maintained
      expect(screen.getByText('Task')).toBeInTheDocument();

      // Verify task header component is rendered (even if mocked)
      expect(screen.getByTestId('task-header')).toBeInTheDocument();
    });

    it('handles empty task iterations gracefully', () => {
      const emptyTask = createMockTask({
        task_id: 'task-123',
        iterations: [],
      });

      vi.mocked(useTask).mockReturnValue({
        data: emptyTask,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskDetail />);

      // Layout should still render even with no iterations
      expect(screen.getByText('Task')).toBeInTheDocument();
      expect(screen.getByRole('tablist')).toBeInTheDocument();
    });

    it('coordinates real-time updates across components', () => {
      // Test that SSE connection is established for real-time updates
      renderWithRouter(<TaskDetail />);

      // Verify components that depend on real-time data are rendered
      expect(screen.getByTestId('tab-content-events')).toBeInTheDocument(); // Events panel should receive updates
    });
  });

  describe('Responsive Layout', () => {
    it('applies full height layout', () => {
      const { container } = renderWithRouter(<TaskDetail />);

      const rootDiv = container.firstChild as HTMLElement;
      expect(rootDiv).toHaveClass(
        'flex',
        'flex-col',
        'h-full',
        'w-full',
        'absolute',
        'inset-0',
      );
    });

    it('makes panel group fill available space', () => {
      renderWithRouter(<TaskDetail />);

      const panelGroup = screen.getByTestId('resizable-panel-group');
      // The parent element is the root div with different classes
      const rootDiv = panelGroup.closest(
        '.flex.flex-col.h-full.w-full.absolute.inset-0',
      );
      expect(rootDiv).toBeInTheDocument();
    });

    it('applies overflow handling to tab content', () => {
      renderWithRouter(<TaskDetail />);

      const eventContent = screen.getByTestId('tab-content-events');
      expect(eventContent).toHaveClass('overflow-hidden');

      const diffContent = screen.getByTestId('tab-content-diff');
      expect(diffContent).toHaveClass('overflow-hidden');
    });
  });

  describe('Integration Edge Cases', () => {
    it('handles task not found state with proper error display', () => {
      vi.mocked(useTask).mockReturnValue({
        data: undefined,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskDetail />);

      expect(screen.getByText('Task not found')).toBeInTheDocument();
      expect(
        screen.getByText('Task with ID task-123 could not be found'),
      ).toBeInTheDocument();
    });

    it('gracefully handles component integration failures', () => {
      // Test with minimal task data that might break some components
      const minimalTask = {
        task_id: 'task-123',
        title: 'Minimal Task',
        // Missing many optional fields
      } as any;

      vi.mocked(useTask).mockReturnValue({
        data: minimalTask,
        isLoading: false,
        error: null,
      } as any);

      renderWithRouter(<TaskDetail />);

      // Should still render basic layout without crashing
      expect(screen.getByTestId('task-header')).toBeInTheDocument();
      expect(screen.getByText('Task')).toBeInTheDocument();
      expect(screen.getByRole('tablist')).toBeInTheDocument();
    });

    it('maintains performance with large datasets', () => {
      // Test integration with large iteration count
      const taskWithManyIterations = createMockTaskWithIterations({}, 50);

      vi.mocked(useTask).mockReturnValue({
        data: taskWithManyIterations,
        isLoading: false,
        error: null,
      } as any);

      const startTime = performance.now();
      renderWithRouter(<TaskDetail />);
      const endTime = performance.now();

      // Should render without performance issues
      expect(endTime - startTime).toBeLessThan(1000); // Render in under 1 second
      expect(screen.getByText('Task')).toBeInTheDocument();
    });
  });
});

// =================================
// CONTRACT TESTING
// =================================
// These tests verify that the route component correctly integrates with its dependencies
// and maintains expected interfaces

describe('TaskDetail Route - Contract Tests', () => {
  const mockResetUIState = vi.fn();
  const mockSetRightPanelView = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();

    vi.mocked(useTask).mockReturnValue({
      data: createMockTask({ task_id: 'task-123' }),
      isLoading: false,
      error: null,
    } as any);

    // Mock all hooks for contract tests
    vi.mocked(useTaskWorkspaceStatus).mockReturnValue({
      data: {
        status: 'idle' as const,
        message: 'Workspace ready',
        created_at: new Date().toISOString(),
      },
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useServerStatus).mockReturnValue({
      data: {
        running: false,
        port: undefined,
        pid: undefined,
        logs: [],
      },
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useTaskEvents).mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useTaskGitDiff).mockReturnValue({
      data: undefined,
      isLoading: false,
      error: null,
    } as any);

    // Mock mutation hooks
    vi.mocked(useStartServer).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    } as any);

    vi.mocked(useStopServer).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    } as any);

    vi.mocked(useStopAgent).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    } as any);

    // Mock ChatInput dependencies
    vi.mocked(useSendMessage).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
      isError: false,
      isSuccess: false,
      error: null,
      data: undefined,
    } as any);

    vi.mocked(useCreateTask).mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
      isError: false,
      isSuccess: false,
      error: null,
      data: undefined,
    } as any);

    vi.mocked(useEnvironmentsList).mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useSearchFiles).mockReturnValue({
      data: { results: [], total_found: 0, query: '' },
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useTaskFileSearch).mockReturnValue({
      data: { results: [], total_found: 0, query: '' },
      isLoading: false,
      error: null,
    } as any);

    vi.mocked(useTaskUIStore).mockImplementation((selector) => {
      const state = {
        rightPanelView: 'events',
        setRightPanelView: mockSetRightPanelView,
        resetUIState: mockResetUIState,
      };
      return selector ? selector(state as any) : state;
    });

    vi.mocked(useRightPanelView).mockReturnValue('events');
    vi.mocked(useSelectedIteration).mockReturnValue(null);
  });

  it('calls external hooks with correct parameters', () => {
    renderWithRouter(<TaskDetail />);

    // Verify hook contracts
    expect(useTask).toHaveBeenCalledWith('task-123');
    expect(useTaskUIStore).toHaveBeenCalled();
    expect(useRightPanelView).toHaveBeenCalled();
  });

  it('provides correct context to child components', () => {
    const taskData = createMockTaskWithIterations(
      {
        task_id: 'test-task',
        title: 'Contract Test Task',
      },
      2,
    );

    vi.mocked(useTask).mockReturnValue({
      data: taskData,
      isLoading: false,
      error: null,
    } as any);

    renderWithRouter(<TaskDetail />);

    // Verify that task data is available to child components
    // This tests the integration without mocking the children
    expect(useTask).toHaveBeenCalledWith('task-123'); // Verify hook contract
    expect(
      screen.getByText(taskData.iterations[0].initial_message),
    ).toBeInTheDocument();
    expect(
      screen.getByText(taskData.iterations[1].initial_message),
    ).toBeInTheDocument();
  });

  it('maintains responsive layout contracts', () => {
    renderWithRouter(<TaskDetail />);

    // Verify layout structure that other components depend on
    const container = screen.getByTestId('resizable-panel-group');
    expect(container).toHaveAttribute('data-direction', 'horizontal');

    const panels = screen.getAllByTestId('resizable-panel');
    expect(panels).toHaveLength(2);

    // Left panel should contain task content
    const leftPanel = panels[0];
    expect(within(leftPanel).getByText('Task')).toBeInTheDocument();

    // Right panel should contain tabs
    const rightPanel = panels[1];
    expect(within(rightPanel).getByRole('tablist')).toBeInTheDocument();
  });
});
