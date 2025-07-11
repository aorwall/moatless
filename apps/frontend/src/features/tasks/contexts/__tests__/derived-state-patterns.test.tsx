import { render, screen, act } from '@testing-library/react'
import { vi } from 'vitest'
import React from 'react'

// Test the derived state patterns to verify they work correctly without useEffect
describe('Derived State Patterns - Regression Tests', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('TaskEventsProvider - Derived streaming status', () => {
    it('should derive streaming status directly from global stream without useEffect', () => {
      let renderCount = 0

      // Mock global stream that changes
      const mockGlobalStream = {
        isConnected: false,
        connectionError: null,
      }

      const TestComponent = () => {
        renderCount++
        
        // This simulates the fixed pattern in task-events-provider.tsx
        const isStreaming = mockGlobalStream.isConnected
        const streamingError = mockGlobalStream.connectionError?.message || null

        return (
          <div>
            <div data-testid="streaming">{isStreaming.toString()}</div>
            <div data-testid="error">{streamingError || 'no-error'}</div>
          </div>
        )
      }

      const { rerender } = render(<TestComponent />)
      
      expect(screen.getByTestId('streaming')).toHaveTextContent('false')
      expect(screen.getByTestId('error')).toHaveTextContent('no-error')
      expect(renderCount).toBe(1)

      // Change global stream state
      mockGlobalStream.isConnected = true
      mockGlobalStream.connectionError = new Error('Connection failed')

      rerender(<TestComponent />)

      expect(screen.getByTestId('streaming')).toHaveTextContent('true')
      expect(screen.getByTestId('error')).toHaveTextContent('Connection failed')
      expect(renderCount).toBe(2)
    })
  })

  describe('Preview Toolbar - Path input state reset', () => {
    it('should reset path input when displayPath changes using render-time pattern', () => {
      let renderCount = 0

      const TestComponent = ({ displayPath }: { displayPath: string }) => {
        renderCount++
        
        const [pathInput, setPathInput] = React.useState(displayPath)
        const [lastDisplayPath, setLastDisplayPath] = React.useState(displayPath)
        
        // This simulates the fixed pattern in preview-toolbar.tsx
        if (displayPath !== lastDisplayPath) {
          setPathInput(displayPath)
          setLastDisplayPath(displayPath)
        }

        return (
          <div>
            <div data-testid="path-input">{pathInput}</div>
            <button 
              data-testid="update-input"
              onClick={() => setPathInput('user-modified')}
            >
              Modify Input
            </button>
          </div>
        )
      }

      const { rerender } = render(<TestComponent displayPath="/initial" />)
      
      expect(screen.getByTestId('path-input')).toHaveTextContent('/initial')

      // User modifies the input
      act(() => {
        screen.getByTestId('update-input').click()
      })
      
      expect(screen.getByTestId('path-input')).toHaveTextContent('user-modified')

      // External navigation changes displayPath - should reset pathInput
      rerender(<TestComponent displayPath="/new-path" />)
      
      expect(screen.getByTestId('path-input')).toHaveTextContent('/new-path')
    })
  })

  describe('Performance: No unnecessary re-renders from derived state', () => {
    it('should not cause extra renders when deriving state from props', () => {
      let renderCount = 0
      let effectCount = 0

      const TestComponent = ({ globalStream }: { 
        globalStream: { isConnected: boolean; connectionError: Error | null } 
      }) => {
        renderCount++
        
        // GOOD: Derive directly (no useEffect)
        const isStreaming = globalStream.isConnected
        const streamingError = globalStream.connectionError?.message || null

        // This effect should only run on mount/unmount, not on every prop change
        React.useEffect(() => {
          effectCount++
          return () => {
            // cleanup
          }
        }, []) // Empty deps - runs once

        return (
          <div>
            <div data-testid="streaming">{isStreaming.toString()}</div>
            <div data-testid="error">{streamingError || 'none'}</div>
          </div>
        )
      }

      const initialStream = { isConnected: false, connectionError: null }
      const { rerender } = render(<TestComponent globalStream={initialStream} />)
      
      expect(renderCount).toBe(1)
      expect(effectCount).toBe(1)

      // Change props multiple times
      rerender(<TestComponent globalStream={{ isConnected: true, connectionError: null }} />)
      rerender(<TestComponent globalStream={{ isConnected: true, connectionError: new Error('failed') }} />)
      rerender(<TestComponent globalStream={{ isConnected: false, connectionError: null }} />)

      // Should render for each prop change, but effect should only run once
      expect(renderCount).toBe(4)
      expect(effectCount).toBe(1) // Still only called once (no useEffect dependency on props)
    })
  })

  describe('Comparison: useEffect anti-pattern vs derived state', () => {
    it('should demonstrate the performance difference', () => {
      let goodRenderCount = 0
      let badRenderCount = 0
      let badEffectCount = 0

      // BAD: Using useEffect to sync state (anti-pattern)
      const BadComponent = ({ isConnected }: { isConnected: boolean }) => {
        badRenderCount++
        const [isStreaming, setIsStreaming] = React.useState(false)

        React.useEffect(() => {
          badEffectCount++
          setIsStreaming(isConnected)
        }, [isConnected]) // This causes extra renders

        return <div data-testid="bad-streaming">{isStreaming.toString()}</div>
      }

      // GOOD: Derive directly (the fixed pattern)
      const GoodComponent = ({ isConnected }: { isConnected: boolean }) => {
        goodRenderCount++
        const isStreaming = isConnected // Direct derivation

        return <div data-testid="good-streaming">{isStreaming.toString()}</div>
      }

      const TestWrapper = ({ isConnected }: { isConnected: boolean }) => (
        <div>
          <BadComponent isConnected={isConnected} />
          <GoodComponent isConnected={isConnected} />
        </div>
      )

      const { rerender } = render(<TestWrapper isConnected={false} />)
      
      // Initial render
      expect(goodRenderCount).toBe(1)
      expect(badRenderCount).toBe(1)
      expect(badEffectCount).toBe(1)

      // Change prop
      act(() => {
        rerender(<TestWrapper isConnected={true} />)
      })

      // Good component: 1 render per prop change
      expect(goodRenderCount).toBe(2)
      
      // Bad component: extra render from useEffect setState
      expect(badRenderCount).toBe(3) // Initial + prop change + effect setState
      expect(badEffectCount).toBe(2)

      // Both should show the correct final value
      expect(screen.getByTestId('good-streaming')).toHaveTextContent('true')
      expect(screen.getByTestId('bad-streaming')).toHaveTextContent('true')
    })
  })
})