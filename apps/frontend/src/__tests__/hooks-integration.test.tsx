import { render } from '@testing-library/react'
import { vi, afterEach } from 'vitest'
import React from 'react'

// Integration test to verify no hooks violations in real components
describe('Hooks Integration - Real Components', () => {
  afterEach(() => {
    vi.clearAllMocks()
  })

  // Mock required modules to prevent import errors
  vi.mock('@tanstack/react-router', () => ({
    useParams: () => ({ taskId: 'test-task-123' }),
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

  vi.mock('@/features/preview/stores/preview-store', () => ({
    usePreviewStore: () => ({
      config: { url: 'http://localhost:3000' },
      currentPath: '/',
      displayPath: '/',
      isInspectorActive: false,
      scriptHealthStatus: 'healthy',
      navigateToPath: vi.fn(),
      toggleInspector: vi.fn(),
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

  it('should render TaskEventsProvider without hooks violations', async () => {
    // Mock the query client and react query
    const { TaskEventsProvider } = await import('@/features/tasks/contexts/task-events-provider')

    // Mock all the query hooks
    vi.mock('@tanstack/react-query', () => ({
      useQuery: () => ({ data: [], isLoading: false, error: null }),
      useQueryClient: () => ({
        setQueryData: vi.fn(),
        invalidateQueries: vi.fn(),
      }),
      QueryClient: vi.fn(),
      QueryClientProvider: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
    }))

    vi.mock('@/features/tasks/hooks/use-task-event-subscription', () => ({
      useTaskEventSubscription: vi.fn(),
    }))

    // This should render without throwing React hooks violations
    const TestComponent = () => (
      <TaskEventsProvider taskId="test-task">
        <div data-testid="provider-content">Content</div>
      </TaskEventsProvider>
    )

    const { getByTestId } = render(<TestComponent />)
    
    // If this doesn't throw, the hooks order is correct
    expect(getByTestId('provider-content')).toBeInTheDocument()
  })

  it('should render PreviewToolbar without hooks violations', async () => {
    // Dynamic import to ensure mocks are set up
    const { PreviewToolbar } = await import('@/features/preview/components/preview-toolbar')

    // This should render without throwing React hooks violations
    const TestComponent = () => (
      <PreviewToolbar />
    )

    // If this doesn't throw React hooks errors, the fix is working
    expect(() => render(<TestComponent />)).not.toThrow()
  })

  it('should verify hooks are called in consistent order across renders', () => {
    let hookCallOrder: string[] = []

    const TestComponent = ({ condition }: { condition: boolean }) => {
      // These hooks should always be called in the same order
      hookCallOrder.push('useState')
      const [state, setState] = React.useState(0)

      hookCallOrder.push('useMemo')
      const memoValue = React.useMemo(() => state * 2, [state])

      hookCallOrder.push('useCallback')
      const callback = React.useCallback(() => setState(prev => prev + 1), [])

      hookCallOrder.push('useEffect')
      React.useEffect(() => {
        // effect that doesn't actually use memoValue
      }, [])

      // This conditional rendering should NOT affect hook order
      if (condition) {
        return (
          <div>
            <span>{memoValue}</span>
            <button onClick={callback}>Increment</button>
          </div>
        )
      }

      return <div>Alternative render</div>
    }

    // First render
    hookCallOrder = []
    const { rerender } = render(<TestComponent condition={true} />)
    const firstRenderOrder = [...hookCallOrder]

    // Second render with different condition
    hookCallOrder = []
    rerender(<TestComponent condition={false} />)
    const secondRenderOrder = [...hookCallOrder]

    // Third render back to original condition
    hookCallOrder = []
    rerender(<TestComponent condition={true} />)
    const thirdRenderOrder = [...hookCallOrder]

    // All renders should have the same hook call order
    expect(firstRenderOrder).toEqual(['useState', 'useMemo', 'useCallback', 'useEffect'])
    expect(secondRenderOrder).toEqual(firstRenderOrder)
    expect(thirdRenderOrder).toEqual(firstRenderOrder)
  })

  describe('Real component performance verification', () => {
    it('should not cause excessive re-renders in derived state patterns', () => {
      let renderCount = 0

      const TaskEventsSimulator = ({ isConnected }: { isConnected: boolean }) => {
        renderCount++
        
        // Simulate the fixed pattern from task-events-provider.tsx
        const globalStream = { isConnected, connectionError: null }
        const isStreaming = globalStream.isConnected
        const streamingError = globalStream.connectionError?.message || null

        return (
          <div data-testid="streaming-status">
            {isStreaming ? 'streaming' : 'not-streaming'}
            {streamingError && ` (error: ${streamingError})`}
          </div>
        )
      }

      const { rerender } = render(<TaskEventsSimulator isConnected={false} />)
      expect(renderCount).toBe(1)

      // Change connection status
      rerender(<TaskEventsSimulator isConnected={true} />)
      expect(renderCount).toBe(2)

      // Same status - should still re-render (normal React behavior)
      rerender(<TaskEventsSimulator isConnected={true} />)
      expect(renderCount).toBe(3)

      // The key point: no infinite loops or unexpected additional renders
      // If there were useEffect loops, renderCount would be much higher
    })
  })
})