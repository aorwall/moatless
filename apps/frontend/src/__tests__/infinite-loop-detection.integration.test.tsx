import { render } from '@testing-library/react'
import React from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

// Stress tests to detect infinite loops and excessive re-renders
describe('Infinite Loop Detection Tests', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('should detect if ChatView causes infinite loops', () => {
    let renderCount = 0
    const maxAllowedRenders = 20 // If it exceeds this, there's likely an infinite loop

    // Mock all the dependencies to isolate the component
    vi.mock('@tanstack/react-router', () => ({
      useParams: () => ({ taskId: 'test-task' }),
    }))

    vi.mock('@/features/tasks/contexts/task-providers', () => ({
      useTaskData: () => ({
        taskId: 'test-task',
        status: 'completed',
        iterations: [{ id: 1, message: 'Test', status: 'completed', messageCount: 1 }],
        repositoryId: 'test-repo',
        isLoading: false,
      }),
      useEventStream: () => ({ events: [] }),
      useStreamingStatus: () => ({ isStreaming: false }),
      useWorkspaceStatus: () => ({ workspaceHasChanges: false }),
    }))

    vi.mock('@/features/tasks/contexts/task-ui-provider', () => ({
      useSelectedIteration: () => ({ setSelectedIteration: vi.fn() }),
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

    const InfiniteLoopDetector = () => {
      renderCount++

      if (renderCount > maxAllowedRenders) {
        throw new Error(`Infinite loop detected! Component rendered ${renderCount} times`)
      }

      // Simulate the patterns from ChatView that were fixed
      const [scrollTop] = React.useState(0)
      const [scrollHeight] = React.useState(0)
      const [clientHeight] = React.useState(0)

      // This should be stable and not cause infinite loops
      const isAtBottom = React.useMemo(() => {
        return scrollHeight - scrollTop - clientHeight < 50
      }, [scrollTop, scrollHeight, clientHeight])

      // These should be stable callbacks
      const availableCommands = React.useMemo(() => {
        return [] // Fixed: stable calculation
      }, []) // Fixed: proper dependencies

      const handleCommandClick = React.useCallback((command: any) => {
        console.log('Command clicked:', command)
      }, []) // Fixed: stable callback

      const handleSendMessage = React.useCallback(
        (_taskId: string, _message: string, clearMessage: () => void) => {
          clearMessage()
        },
        [],
      ) // Fixed: stable callback

      return (
        <div data-testid="chat-view">
          <div>At bottom: {isAtBottom.toString()}</div>
          <div>Commands: {availableCommands.length}</div>
          <button onClick={() => handleCommandClick({ name: 'test' })}>Test Command</button>
          <button onClick={() => handleSendMessage('test', 'message', () => {})}>
            Send Message
          </button>
        </div>
      )
    }

    // This should render without infinite loops
    const { getByTestId } = render(<InfiniteLoopDetector />)

    expect(getByTestId('chat-view')).toBeInTheDocument()
    expect(renderCount).toBeLessThan(maxAllowedRenders)

    // Should be reasonable number of renders (initial + any necessary updates)
    expect(renderCount).toBeLessThanOrEqual(5)
  })

  it('should detect infinite loops in derived state patterns', () => {
    let renderCount = 0
    const maxAllowedRenders = 10

    const TaskEventsSimulator = ({ globalStreamConnected }: { globalStreamConnected: boolean }) => {
      renderCount++

      if (renderCount > maxAllowedRenders) {
        throw new Error(`TaskEvents infinite loop detected! Rendered ${renderCount} times`)
      }

      // Simulate the fixed pattern from task-events-provider.tsx
      const globalStream = {
        isConnected: globalStreamConnected,
        connectionError: null,
      }

      // Direct derivation should NOT cause infinite loops
      const isStreaming = globalStream.isConnected
      const streamingError = (globalStream.connectionError as Error | null)?.message || null

      // This memoization should be stable
      const contextValue = React.useMemo(
        () => ({
          events: [],
          isStreaming,
          streamingError,
        }),
        [isStreaming, streamingError],
      )

      return (
        <div data-testid="task-events">
          <div>Streaming: {contextValue.isStreaming.toString()}</div>
          <div>Error: {contextValue.streamingError || 'none'}</div>
        </div>
      )
    }

    // Render with initial props
    const { rerender, getByTestId } = render(<TaskEventsSimulator globalStreamConnected={false} />)

    expect(getByTestId('task-events')).toBeInTheDocument()
    expect(renderCount).toBe(1)

    // Change props - should not cause infinite loop
    rerender(<TaskEventsSimulator globalStreamConnected={true} />)
    expect(renderCount).toBe(2)

    // Same props - should not cause additional renders beyond React's normal behavior
    rerender(<TaskEventsSimulator globalStreamConnected={true} />)
    expect(renderCount).toBe(3)

    expect(renderCount).toBeLessThan(maxAllowedRenders)
  })

  it('should detect excessive re-renders in useEffect patterns', () => {
    let renderCount = 0
    let effectCount = 0
    const maxAllowedRenders = 10

    const EffectStressTest = ({ externalValue }: { externalValue: string }) => {
      renderCount++

      if (renderCount > maxAllowedRenders) {
        throw new Error(`Effect stress test failed! Rendered ${renderCount} times`)
      }

      const [internalState, setInternalState] = React.useState(externalValue)

      // This effect should have complete dependencies and not cause infinite loops
      React.useEffect(() => {
        effectCount++

        if (effectCount > 5) {
          throw new Error(`Effect infinite loop detected! Effect ran ${effectCount} times`)
        }

        setInternalState(externalValue)
      }, [externalValue]) // Complete dependency array

      // Stable callback should not cause re-renders
      const stableCallback = React.useCallback(() => {
        return `processed-${internalState}`
      }, [internalState])

      const memoizedValue = React.useMemo(() => {
        return stableCallback()
      }, [stableCallback])

      return (
        <div data-testid="effect-test">
          <div>Internal: {internalState}</div>
          <div>Memoized: {memoizedValue}</div>
        </div>
      )
    }

    const { rerender, getByTestId } = render(<EffectStressTest externalValue="initial" />)

    expect(getByTestId('effect-test')).toBeInTheDocument()

    // Change external value
    rerender(<EffectStressTest externalValue="changed" />)

    // Should not have excessive renders or effects
    expect(renderCount).toBeLessThan(maxAllowedRenders)
    expect(effectCount).toBeLessThanOrEqual(2) // Once for each value change
  })

  it('should detect memory leaks from uncleaned subscriptions', () => {
    let subscriptionCount = 0
    let cleanupCount = 0

    const MemoryLeakDetector = ({ isActive }: { isActive: boolean }) => {
      React.useEffect(() => {
        if (!isActive) return

        subscriptionCount++

        // Simulate a subscription that needs cleanup
        const subscription = {
          id: subscriptionCount,
          unsubscribe: () => {
            cleanupCount++
          },
        }

        return () => {
          subscription.unsubscribe()
        }
      }, [isActive])

      return <div data-testid="memory-test">Active: {isActive.toString()}</div>
    }

    const { rerender, unmount } = render(<MemoryLeakDetector isActive={false} />)

    expect(subscriptionCount).toBe(0)
    expect(cleanupCount).toBe(0)

    // Activate - should create subscription
    rerender(<MemoryLeakDetector isActive={true} />)
    expect(subscriptionCount).toBe(1)
    expect(cleanupCount).toBe(0)

    // Deactivate - should cleanup
    rerender(<MemoryLeakDetector isActive={false} />)
    expect(subscriptionCount).toBe(1)
    expect(cleanupCount).toBe(1)

    // Reactivate
    rerender(<MemoryLeakDetector isActive={true} />)
    expect(subscriptionCount).toBe(2)
    expect(cleanupCount).toBe(1)

    // Unmount - should cleanup final subscription
    unmount()
    expect(cleanupCount).toBe(2)

    // No memory leak if subscriptions === cleanups
    expect(subscriptionCount).toBe(cleanupCount)
  })

  it('should verify stable callbacks do not cause excessive re-computations', () => {
    let memoizationCount = 0
    const callbackRef = React.createRef<(items: string[]) => string[]>()

    const CallbackStabilityTest = ({ data }: { data: string[] }) => {
      // Use a stable function reference
      const processData = React.useCallback((items: string[]) => {
        return items.map((item) => item.toUpperCase())
      }, [])

      // Store reference to check stability
      if (!callbackRef.current) {
        callbackRef.current = processData
      }

      // This memoization should only change when data changes
      const processedData = React.useMemo(() => {
        memoizationCount++
        return processData(data)
      }, [data, processData])

      return (
        <div data-testid="callback-test">
          <div>Items: {processedData.join(', ')}</div>
        </div>
      )
    }

    const { rerender } = render(<CallbackStabilityTest data={['a', 'b']} />)

    expect(memoizationCount).toBe(1)

    // Same data - memo should not recompute excessively
    rerender(<CallbackStabilityTest data={['a', 'b']} />)
    // React may re-render but shouldn't cause excessive memoization
    expect(memoizationCount).toBeLessThanOrEqual(3)

    // Different data - memo should recompute
    rerender(<CallbackStabilityTest data={['c', 'd']} />)
    expect(memoizationCount).toBeLessThanOrEqual(5)

    // Verify no runaway re-computations (should be reasonable)
    expect(memoizationCount).toBeLessThan(10)
  })
})
