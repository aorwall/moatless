import { render, act } from '@testing-library/react'
import { vi } from 'vitest'
import React from 'react'

// Performance regression tests for React hooks fixes
describe('Hooks Performance Regression Tests', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  describe('Stable callback dependencies', () => {
    it('should not cause infinite re-renders with useCallback dependencies', () => {
      let renderCount = 0
      let callbackCreationCount = 0

      const TestComponent = ({ externalDep }: { externalDep: string }) => {
        renderCount++

        // Simulate a stable callback that was causing infinite loops
        const stableCallback = React.useCallback(() => {
          callbackCreationCount++
          return `processed-${externalDep}`
        }, [externalDep]) // Fixed: proper dependencies

        const memoizedValue = React.useMemo(() => {
          return stableCallback()
        }, [stableCallback]) // This should be stable when externalDep doesn't change

        return <div data-testid="value">{memoizedValue}</div>
      }

      const { rerender } = render(<TestComponent externalDep="test" />)
      
      expect(renderCount).toBe(1)
      expect(callbackCreationCount).toBe(1)

      // Same props - should not re-render or recreate callback
      rerender(<TestComponent externalDep="test" />)
      
      expect(renderCount).toBe(2) // React re-renders but callback is stable
      expect(callbackCreationCount).toBe(1) // Callback not recreated

      // Different props - should recreate callback
      rerender(<TestComponent externalDep="different" />)
      
      expect(renderCount).toBe(3)
      expect(callbackCreationCount).toBe(2) // Callback recreated due to dependency change
    })
  })

  describe('useEffect dependency completeness', () => {
    it('should run effects when dependencies change but not cause infinite loops', () => {
      let effectRunCount = 0
      let renderCount = 0

      const TestComponent = ({ value, callback }: { 
        value: string
        callback: (val: string) => void 
      }) => {
        renderCount++
        const [result, setResult] = React.useState('')

        // Fixed: Complete dependency array prevents stale closures
        React.useEffect(() => {
          effectRunCount++
          if (value) {
            callback(value)
            setResult(`processed-${value}`)
          }
        }, [value, callback]) // Complete dependencies

        return <div data-testid="result">{result}</div>
      }

      const mockCallback = vi.fn()
      const { rerender } = render(
        <TestComponent value="initial" callback={mockCallback} />
      )
      
      expect(effectRunCount).toBe(1)
      expect(renderCount).toBe(2) // Initial + effect setState
      expect(mockCallback).toHaveBeenCalledWith('initial')

      // Same value and callback - effect should not run again
      rerender(<TestComponent value="initial" callback={mockCallback} />)
      
      expect(effectRunCount).toBe(1) // No additional effect run
      expect(renderCount).toBe(3) // Just the re-render

      // Different value - effect should run
      rerender(<TestComponent value="updated" callback={mockCallback} />)
      
      expect(effectRunCount).toBe(2) // Effect runs again
      expect(mockCallback).toHaveBeenCalledWith('updated')
    })
  })

  describe('Derived state vs useEffect performance', () => {
    it('should demonstrate performance improvement of derived state', () => {
      let derivedRenderCount = 0
      let effectRenderCount = 0
      let effectCallCount = 0

      // Component using derived state (optimized)
      const DerivedStateComponent = ({ 
        items 
      }: { 
        items: string[] 
      }) => {
        derivedRenderCount++
        
        // Direct derivation - no useEffect needed
        const filteredItems = React.useMemo(() => {
          return items.filter(item => item.startsWith('valid'))
        }, [items])

        const itemCount = filteredItems.length

        return (
          <div data-testid="derived">
            Count: {itemCount}
          </div>
        )
      }

      // Component using useEffect anti-pattern (not optimized)
      const EffectStateComponent = ({ 
        items 
      }: { 
        items: string[] 
      }) => {
        effectRenderCount++
        const [itemCount, setItemCount] = React.useState(0)

        React.useEffect(() => {
          effectCallCount++
          const filtered = items.filter(item => item.startsWith('valid'))
          setItemCount(filtered.length)
        }, [items])

        return (
          <div data-testid="effect">
            Count: {itemCount}
          </div>
        )
      }

      const TestWrapper = ({ items }: { items: string[] }) => (
        <div>
          <DerivedStateComponent items={items} />
          <EffectStateComponent items={items} />
        </div>
      )

      const initialItems = ['valid1', 'invalid', 'valid2']
      const { rerender } = render(<TestWrapper items={initialItems} />)

      // Initial state
      expect(derivedRenderCount).toBe(1)
      expect(effectRenderCount).toBe(2) // Initial render + effect setState
      expect(effectCallCount).toBe(1)

      // Change items
      const newItems = ['valid3', 'valid4', 'invalid2']
      act(() => {
        rerender(<TestWrapper items={newItems} />)
      })

      // Performance comparison
      expect(derivedRenderCount).toBe(2) // Only renders when props change
      expect(effectRenderCount).toBe(3) // Initial + effect + props change
      expect(effectCallCount).toBe(2)

      // Both should produce correct results despite performance difference
      // (In a real test, you'd check the rendered content)
    })
  })

  describe('Memory leaks prevention', () => {
    it('should not accumulate event listeners or subscriptions', () => {
      let subscriptionCount = 0
      let cleanupCount = 0

      const TestComponent = ({ isActive }: { isActive: boolean }) => {
        React.useEffect(() => {
          if (!isActive) return

          subscriptionCount++
          const listener = () => console.log('event')
          
          // Simulate subscription
          window.addEventListener('test-event', listener)
          
          return () => {
            cleanupCount++
            window.removeEventListener('test-event', listener)
          }
        }, [isActive]) // Proper dependencies

        return <div>Active: {isActive.toString()}</div>
      }

      const { rerender, unmount } = render(<TestComponent isActive={false} />)
      
      expect(subscriptionCount).toBe(0)
      expect(cleanupCount).toBe(0)

      // Activate
      rerender(<TestComponent isActive={true} />)
      expect(subscriptionCount).toBe(1)
      expect(cleanupCount).toBe(0)

      // Deactivate
      rerender(<TestComponent isActive={false} />)
      expect(subscriptionCount).toBe(1) // No new subscription
      expect(cleanupCount).toBe(1) // Cleanup called

      // Activate again
      rerender(<TestComponent isActive={true} />)
      expect(subscriptionCount).toBe(2) // New subscription
      expect(cleanupCount).toBe(1)

      // Unmount - should cleanup
      unmount()
      expect(cleanupCount).toBe(2)
    })
  })
})