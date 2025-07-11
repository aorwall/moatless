import { render, screen } from '@testing-library/react'
import { vi } from 'vitest'
import React from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'

// Test actual components to ensure they don't crash with hooks errors
describe('Component Crash Detection Tests', () => {
  let queryClient: QueryClient

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    })
    vi.clearAllMocks()
  })

  // Comprehensive mocking for all dependencies
  const setupMocks = () => {
    vi.mock('@tanstack/react-router', () => ({
      useParams: () => ({ taskId: 'test-task-123', repositoryId: 'test-repo' }),
      MemoryRouter: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
    }))

    vi.mock('@/features/events/contexts/global-event-stream-provider', () => ({
      useGlobalEventStream: () => ({
        isConnected: false,
        connectionError: null,
        subscribe: vi.fn(),
        unsubscribe: vi.fn(),
      }),
    }))

    vi.mock('@/features/tasks/contexts/task-ui-provider', () => ({
      useSelectedIteration: () => ({ setSelectedIteration: vi.fn() }),
      useTaskUI: () => ({
        currentState: {
          rightPanelView: 'events',
          selectedIteration: null,
          panelSizes: [50, 50],
          selectedFile: null,
          chatState: {},
          gitDiffState: { expandedFiles: new Set(), expandedPaths: new Set() },
        },
        setRightPanelView: vi.fn(),
        setSelectedIteration: vi.fn(),
        setPanelSizes: vi.fn(),
        setSelectedFile: vi.fn(),
        updateChatState: vi.fn(),
        updateGitDiffState: vi.fn(),
      }),
    }))

    vi.mock('@/features/tasks/contexts/task-providers', () => ({
      useTaskData: () => ({
        taskId: 'test-task',
        status: 'completed',
        iterations: [
          {
            id: 1,
            message: 'Test iteration',
            status: 'completed',
            messageCount: 1,
            durationMs: 1000,
            startTime: new Date().toISOString(),
          },
        ],
        repositoryId: 'test-repo',
        isLoading: false,
      }),
      useEventStream: () => ({ events: [] }),
      useStreamingStatus: () => ({ isStreaming: false }),
      useWorkspaceStatus: () => ({ workspaceHasChanges: false }),
    }))

    vi.mock('@/features/tasks/hooks/use-auto-scroll', () => ({
      useAutoScroll: () => ({ scrollRef: { current: null } }),
    }))

    vi.mock('@/features/tasks/hooks/use-tasks', () => ({
      useSendMessage: () => ({ mutate: vi.fn() }),
    }))

    vi.mock('@/features/commands/hooks/use-commands', () => ({
      useCommandsList: () => ({ data: [] }),
    }))

    vi.mock('@/features/preview/stores/preview-store', () => ({
      usePreviewStore: () => ({
        config: { url: 'http://localhost:3000' },
        currentPath: '/',
        displayPath: '/',
        isInspectorActive: false,
        scriptHealthStatus: 'healthy',
        navigateToPath: vi.fn(),
        toggleInspector: vi.fn(),
        setConfig: vi.fn(),
        setLoaded: vi.fn(),
        setError: vi.fn(),
        setDisplayPath: vi.fn(),
        setScriptHealthStatus: vi.fn(),
        updateHeartbeat: vi.fn(),
      }),
    }))

    vi.mock('@/features/preview/hooks/use-server', () => ({
      useServerStatus: () => ({ data: { running: false } }),
      useStartServer: () => ({ mutate: vi.fn() }),
      useStopServer: () => ({ mutate: vi.fn() }),
      useRestartServer: () => ({ mutate: vi.fn() }),
    }))

    vi.mock('@/features/preview/hooks/use-preview-connection', () => ({
      usePreviewConnection: () => ({ isConnected: false }),
    }))

    vi.mock('@/lib/components/theme-provider', () => ({
      useTheme: () => ({ theme: 'light' }),
    }))

    vi.mock('sonner', () => ({
      toast: {
        error: vi.fn(),
        success: vi.fn(),
      },
    }))
  }

  it('should render ChatView without hooks crashes', async () => {
    setupMocks()

    // Mock mermaid to prevent issues
    vi.mock('mermaid', () => ({
      default: {
        initialize: vi.fn(),
        render: vi.fn().mockResolvedValue({ svg: '<svg>test</svg>' }),
      },
    }))

    const TestWrapper = ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={queryClient}>
        <React.Suspense fallback={<div>Loading...</div>}>
          {children}
        </React.Suspense>
      </QueryClientProvider>
    )

    // Import component dynamically to ensure mocks are applied
    const { default: ChatView } = await import('@/features/tasks/components/chat-view/chat-view')

    // This should render without throwing React hooks violations
    expect(() => {
      render(
        <TestWrapper>
          <ChatView />
        </TestWrapper>
      )
    }).not.toThrow()

    // Should render some content
    expect(document.body).toContainHTML('div')
  })

  it('should render PreviewToolbar without hooks crashes', async () => {
    setupMocks()

    const TestWrapper = ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={queryClient}>
        {children}
      </QueryClientProvider>
    )

    const { PreviewToolbar } = await import('@/features/preview/components/preview-toolbar')

    // This should render without throwing React hooks violations
    expect(() => {
      render(
        <TestWrapper>
          <PreviewToolbar />
        </TestWrapper>
      )
    }).not.toThrow()
  })

  it('should render TaskEventsProvider without hooks crashes', async () => {
    setupMocks()

    vi.mock('@/features/tasks/hooks/use-task-event-subscription', () => ({
      useTaskEventSubscription: vi.fn(),
    }))

    vi.mock('@/lib/api/query-keys', () => ({
      queryKeys: {
        taskEvents: () => ['taskEvents'],
      },
    }))

    const TestWrapper = ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={queryClient}>
        {children}
      </QueryClientProvider>
    )

    const { TaskEventsProvider } = await import('@/features/tasks/contexts/task-events-provider')

    // This should render without throwing React hooks violations
    expect(() => {
      render(
        <TestWrapper>
          <TaskEventsProvider taskId="test-task">
            <div data-testid="provider-content">Content</div>
          </TaskEventsProvider>
        </TestWrapper>
      )
    }).not.toThrow()
  })

  it('should render MarkdownPreview without hooks crashes', async () => {
    setupMocks()

    // Mock mermaid library
    vi.mock('mermaid', () => ({
      default: {
        initialize: vi.fn(),
        render: vi.fn().mockResolvedValue({ svg: '<svg>test</svg>' }),
      },
    }))

    const { MarkdownPreview } = await import('@/features/file-viewer/components/markdown-preview')

    const testContent = `
# Test Markdown

This is a test.

\`\`\`mermaid
graph TD
  A --> B
\`\`\`
    `

    // This should render without throwing React hooks violations
    expect(() => {
      render(<MarkdownPreview content={testContent} />)
    }).not.toThrow()
  })

  it('should handle rapid prop changes without hooks crashes', async () => {
    setupMocks()

    const { TaskEventsProvider } = await import('@/features/tasks/contexts/task-events-provider')

    vi.mock('@/features/tasks/hooks/use-task-event-subscription', () => ({
      useTaskEventSubscription: vi.fn(),
    }))

    vi.mock('@/lib/api/query-keys', () => ({
      queryKeys: {
        taskEvents: () => ['taskEvents'],
      },
    }))

    const TestWrapper = ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={queryClient}>
        {children}
      </QueryClientProvider>
    )

    const TestComponent = ({ taskId }: { taskId: string }) => (
      <TestWrapper>
        <TaskEventsProvider taskId={taskId}>
          <div data-testid="provider-content">Task: {taskId}</div>
        </TaskEventsProvider>
      </TestWrapper>
    )

    const { rerender } = render(<TestComponent taskId="task-1" />)

    // Rapid prop changes should not cause hooks order errors
    expect(() => {
      rerender(<TestComponent taskId="task-2" />)
      rerender(<TestComponent taskId="task-3" />)
      rerender(<TestComponent taskId="task-1" />)
      rerender(<TestComponent taskId="task-4" />)
    }).not.toThrow()

    expect(screen.getByTestId('provider-content')).toHaveTextContent('Task: task-4')
  })

  it('should handle edge cases without hooks crashes', () => {
    // Test the derived state patterns with edge cases
    const DerivedStateStressTest = ({ 
      globalStream 
    }: { 
      globalStream: { isConnected: boolean; connectionError: Error | null } | null 
    }) => {
      // Simulate the fixed pattern with potential null values
      const isStreaming = globalStream?.isConnected ?? false
      const streamingError = globalStream?.connectionError?.message || null

      // Test memoization with potential null dependencies
      const contextValue = React.useMemo(() => ({
        isStreaming,
        streamingError,
        events: [],
      }), [isStreaming, streamingError])

      return (
        <div data-testid="derived-state">
          <div>Streaming: {contextValue.isStreaming.toString()}</div>
          <div>Error: {contextValue.streamingError || 'none'}</div>
        </div>
      )
    }

    const { rerender } = render(<DerivedStateStressTest globalStream={null} />)

    expect(screen.getByTestId('derived-state')).toBeInTheDocument()

    // Test various edge cases
    expect(() => {
      rerender(<DerivedStateStressTest globalStream={{ isConnected: true, connectionError: null }} />)
      rerender(<DerivedStateStressTest globalStream={{ isConnected: false, connectionError: new Error('Test') }} />)
      rerender(<DerivedStateStressTest globalStream={null} />)
      rerender(<DerivedStateStressTest globalStream={{ isConnected: true, connectionError: null }} />)
    }).not.toThrow()
  })
})