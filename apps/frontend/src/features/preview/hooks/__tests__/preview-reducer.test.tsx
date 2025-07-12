import { act, renderHook } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ElementInfo, PreviewConfig } from '../../types'
import { PreviewProvider, usePreview } from '../use-preview'

function createWrapper() {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <PreviewProvider>{children}</PreviewProvider>
  }
}

describe('Preview Reducer Logic', () => {
  let wrapper: ReturnType<typeof createWrapper>

  beforeEach(() => {
    wrapper = createWrapper()
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  describe('State Transitions', () => {
    it('should handle complex state transitions correctly', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      // Initial state
      expect(result.current.isLoaded).toBe(false)
      expect(result.current.error).toBe(null)
      expect(result.current.isInspectorActive).toBe(false)
      expect(result.current.selectedElement).toBe(null)

      // Transition 1: Load successfully
      act(() => {
        result.current.setLoaded(true)
      })

      expect(result.current.isLoaded).toBe(true)
      expect(result.current.error).toBe(null)

      // Transition 2: Activate inspector and select element
      const mockElement: ElementInfo = {
        selector: '.test-element',
        displayText: 'Test Element',
        tagName: 'div',
        attributes: {},
        textContent: 'Test content',
        boundingRect: { top: 0, left: 0, width: 100, height: 50 },
      }

      act(() => {
        result.current.toggleInspector()
        result.current.setSelectedElement(mockElement)
      })

      expect(result.current.isInspectorActive).toBe(true)
      expect(result.current.selectedElement).toEqual(mockElement)

      // Transition 3: Error occurs - should set loaded to false but preserve inspector state
      act(() => {
        result.current.setError('Connection failed')
      })

      expect(result.current.isLoaded).toBe(false)
      expect(result.current.error).toBe('Connection failed')
      expect(result.current.isInspectorActive).toBe(true) // Should remain true
      expect(result.current.selectedElement).toEqual(mockElement) // Should remain

      // Transition 4: Recover by loading again - should clear error
      act(() => {
        result.current.setLoaded(true)
      })

      expect(result.current.isLoaded).toBe(true)
      expect(result.current.error).toBe(null)
      expect(result.current.isInspectorActive).toBe(true)
      expect(result.current.selectedElement).toEqual(mockElement)

      // Transition 5: Deactivate inspector - should clear selected element
      act(() => {
        result.current.toggleInspector()
      })

      expect(result.current.isInspectorActive).toBe(false)
      expect(result.current.selectedElement).toBe(null)
    })

    it('should handle rapid state changes correctly', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      // Rapid configuration changes
      act(() => {
        result.current.setConfig({ url: 'http://localhost:3000' })
        result.current.setConfig({ url: 'http://localhost:4000' })
        result.current.setConfig({ url: 'http://localhost:5000' })
      })

      expect(result.current.config.url).toBe('http://localhost:5000')

      // Rapid path changes
      act(() => {
        result.current.setCurrentPath('/page1')
        result.current.setCurrentPath('/page2')
        result.current.navigateToPath('/page3')
      })

      expect(result.current.currentPath).toBe('/page3')
      expect(result.current.displayPath).toBe('/page3')

      // Rapid inspector toggles
      act(() => {
        result.current.toggleInspector() // true
        result.current.toggleInspector() // false
        result.current.toggleInspector() // true
      })

      expect(result.current.isInspectorActive).toBe(true)
    })
  })

  describe('Edge Cases', () => {
    it('should handle inspector toggle with existing selected element', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })
      const mockElement: ElementInfo = {
        selector: '.test-element',
        displayText: 'Test Element',
        tagName: 'div',
        attributes: {},
        textContent: 'Test content',
        boundingRect: { top: 0, left: 0, width: 100, height: 50 },
      }

      // Set element while inspector is inactive
      act(() => {
        result.current.setSelectedElement(mockElement)
      })

      expect(result.current.selectedElement).toEqual(mockElement)
      expect(result.current.isInspectorActive).toBe(false)

      // Activate inspector - element should remain
      act(() => {
        result.current.toggleInspector()
      })

      expect(result.current.isInspectorActive).toBe(true)
      expect(result.current.selectedElement).toEqual(mockElement)

      // Deactivate inspector - element should be cleared
      act(() => {
        result.current.toggleInspector()
      })

      expect(result.current.isInspectorActive).toBe(false)
      expect(result.current.selectedElement).toBe(null)
    })

    it('should handle setting error when already loaded', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      // First set loaded
      act(() => {
        result.current.setLoaded(true)
      })

      expect(result.current.isLoaded).toBe(true)
      expect(result.current.error).toBe(null)

      // Then set error
      act(() => {
        result.current.setError('Test error')
      })

      expect(result.current.isLoaded).toBe(false)
      expect(result.current.error).toBe('Test error')
    })

    it('should handle setting loaded when there is an error', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      // First set error
      act(() => {
        result.current.setError('Test error')
      })

      expect(result.current.isLoaded).toBe(false)
      expect(result.current.error).toBe('Test error')

      // Then set loaded
      act(() => {
        result.current.setLoaded(true)
      })

      expect(result.current.isLoaded).toBe(true)
      expect(result.current.error).toBe(null)
    })

    it('should handle clearing error by setting null', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      // Set error
      act(() => {
        result.current.setError('Test error')
      })

      expect(result.current.error).toBe('Test error')

      // Clear error
      act(() => {
        result.current.setError(null)
      })

      expect(result.current.error).toBe(null)
    })
  })

  describe('Heartbeat and Script Health Logic', () => {
    it('should update heartbeat timestamp correctly', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })
      const beforeTime = Date.now()

      act(() => {
        result.current.updateHeartbeat()
      })

      const afterTime = Date.now()

      expect(result.current.lastHeartbeat).toBeGreaterThanOrEqual(beforeTime)
      expect(result.current.lastHeartbeat).toBeLessThanOrEqual(afterTime)
      expect(result.current.scriptHealthStatus).toBe('injected-reachable')
    })

    it('should maintain separate script health status updates', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      // Set status manually
      act(() => {
        result.current.setScriptHealthStatus('injected-not-reachable')
      })

      expect(result.current.scriptHealthStatus).toBe('injected-not-reachable')

      // Update heartbeat should override status
      act(() => {
        result.current.updateHeartbeat()
      })

      expect(result.current.scriptHealthStatus).toBe('injected-reachable')

      // Set status manually again
      act(() => {
        result.current.setScriptHealthStatus('not-injected')
      })

      expect(result.current.scriptHealthStatus).toBe('not-injected')
    })

    it('should handle multiple heartbeat updates', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      let lastHeartbeat = 0

      act(() => {
        result.current.updateHeartbeat()
      })

      lastHeartbeat = result.current.lastHeartbeat!
      expect(result.current.lastHeartbeat).toBe(lastHeartbeat)

      // Advance time slightly
      vi.advanceTimersByTime(100)

      act(() => {
        result.current.updateHeartbeat()
      })

      expect(result.current.lastHeartbeat).toBeGreaterThan(lastHeartbeat)
    })
  })

  describe('Path Normalization', () => {
    it('should normalize paths consistently', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      const testCases = [
        { input: 'dashboard', expected: '/dashboard' },
        { input: '/dashboard', expected: '/dashboard' },
        { input: 'users/profile', expected: '/users/profile' },
        { input: '/users/profile', expected: '/users/profile' },
        { input: '', expected: '/' },
        { input: 'home/about', expected: '/home/about' },
      ]

      for (const { input, expected } of testCases) {
        act(() => {
          result.current.setCurrentPath(input)
        })
        expect(result.current.currentPath).toBe(expected)

        act(() => {
          result.current.setDisplayPath(input)
        })
        expect(result.current.displayPath).toBe(expected)

        act(() => {
          result.current.navigateToPath(input)
        })
        expect(result.current.currentPath).toBe(expected)
        expect(result.current.displayPath).toBe(expected)
      }
    })
  })

  describe('Configuration Immutability', () => {
    it('should create new config objects on updates', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })
      const initialConfig = result.current.config

      const newConfig: PreviewConfig = { url: 'http://localhost:4000' }

      act(() => {
        result.current.setConfig(newConfig)
      })

      expect(result.current.config).not.toBe(initialConfig)
      expect(result.current.config).toEqual(newConfig)
    })

    it('should handle config object references correctly', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      const config1: PreviewConfig = { url: 'http://localhost:4000' }
      const config2: PreviewConfig = { url: 'http://localhost:5000' }

      act(() => {
        result.current.setConfig(config1)
      })

      expect(result.current.config).toEqual(config1)

      act(() => {
        result.current.setConfig(config2)
      })

      expect(result.current.config).toEqual(config2)
      expect(result.current.config).not.toEqual(config1)
    })
  })

  describe('State Consistency', () => {
    it('should maintain state consistency across multiple operations', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      // Perform a series of operations
      act(() => {
        result.current.setConfig({ url: 'http://localhost:4000' })
        result.current.navigateToPath('/dashboard')
        result.current.setConnected(true)
        result.current.setLoaded(true)
        result.current.toggleInspector()
        result.current.setScriptHealthStatus('injected-reachable')
        result.current.updateHeartbeat()
      })

      // Verify all state is as expected
      expect(result.current.config.url).toBe('http://localhost:4000')
      expect(result.current.currentPath).toBe('/dashboard')
      expect(result.current.displayPath).toBe('/dashboard')
      expect(result.current.isConnected).toBe(true)
      expect(result.current.isLoaded).toBe(true)
      expect(result.current.isInspectorActive).toBe(true)
      expect(result.current.scriptHealthStatus).toBe('injected-reachable')
      expect(result.current.lastHeartbeat).toBeTypeOf('number')
      expect(result.current.error).toBe(null)

      // Reset should clear everything
      act(() => {
        result.current.reset()
      })

      // Verify reset worked
      expect(result.current.config).toEqual({ url: 'http://localhost:3000' })
      expect(result.current.currentPath).toBe('/')
      expect(result.current.displayPath).toBe('/')
      expect(result.current.isConnected).toBe(false)
      expect(result.current.isLoaded).toBe(false)
      expect(result.current.isInspectorActive).toBe(false)
      expect(result.current.scriptHealthStatus).toBe('not-injected')
      expect(result.current.lastHeartbeat).toBe(null)
      expect(result.current.error).toBe(null)
      expect(result.current.selectedElement).toBe(null)
    })
  })
})
