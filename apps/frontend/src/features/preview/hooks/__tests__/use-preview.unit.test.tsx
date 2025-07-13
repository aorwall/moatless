import { act, renderHook } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { PreviewConfig, ScriptHealthStatus } from '../../types'
import { PreviewProvider, usePreview } from '../use-preview'
import { createMockElementInfo } from './test-helpers'

// Test wrapper component
function createWrapper() {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <PreviewProvider>{children}</PreviewProvider>
  }
}

describe('usePreview', () => {
  let wrapper: ReturnType<typeof createWrapper>

  beforeEach(() => {
    wrapper = createWrapper()
  })

  describe('Initial State', () => {
    it('should have correct initial state', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      expect(result.current.config).toEqual({ url: 'http://localhost:3000' })
      expect(result.current.currentPath).toBe('/')
      expect(result.current.displayPath).toBe('/')
      expect(result.current.isInspectorActive).toBe(false)
      expect(result.current.isLoaded).toBe(false)
      expect(result.current.selectedElement).toBe(null)
      expect(result.current.error).toBe(null)
      expect(result.current.isConnected).toBe(false)
      expect(result.current.scriptHealthStatus).toBe('not-injected')
      expect(result.current.lastHeartbeat).toBe(null)
    })

    it('should provide all required action functions', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      expect(typeof result.current.setConfig).toBe('function')
      expect(typeof result.current.setCurrentPath).toBe('function')
      expect(typeof result.current.setDisplayPath).toBe('function')
      expect(typeof result.current.navigateToPath).toBe('function')
      expect(typeof result.current.toggleInspector).toBe('function')
      expect(typeof result.current.setSelectedElement).toBe('function')
      expect(typeof result.current.setError).toBe('function')
      expect(typeof result.current.setConnected).toBe('function')
      expect(typeof result.current.setLoaded).toBe('function')
      expect(typeof result.current.setScriptHealthStatus).toBe('function')
      expect(typeof result.current.updateHeartbeat).toBe('function')
      expect(typeof result.current.reset).toBe('function')
    })
  })

  describe('Configuration Management', () => {
    it('should update config', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })
      const newConfig: PreviewConfig = { url: 'http://localhost:4000' }

      act(() => {
        result.current.setConfig(newConfig)
      })

      expect(result.current.config).toEqual(newConfig)
    })
  })

  describe('Path Management', () => {
    it('should update current path with normalization', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      act(() => {
        result.current.setCurrentPath('home')
      })

      expect(result.current.currentPath).toBe('/home')
    })

    it('should keep paths starting with / unchanged', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      act(() => {
        result.current.setCurrentPath('/dashboard')
      })

      expect(result.current.currentPath).toBe('/dashboard')
    })

    it('should update display path with normalization', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      act(() => {
        result.current.setDisplayPath('about')
      })

      expect(result.current.displayPath).toBe('/about')
    })

    it('should update both paths when navigating', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      act(() => {
        result.current.navigateToPath('products')
      })

      expect(result.current.currentPath).toBe('/products')
      expect(result.current.displayPath).toBe('/products')
    })
  })

  describe('Inspector Management', () => {
    it('should toggle inspector active state', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      expect(result.current.isInspectorActive).toBe(false)

      act(() => {
        result.current.toggleInspector()
      })

      expect(result.current.isInspectorActive).toBe(true)

      act(() => {
        result.current.toggleInspector()
      })

      expect(result.current.isInspectorActive).toBe(false)
    })

    it('should clear selected element when deactivating inspector', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })
      const mockElement = createMockElementInfo()

      // Activate inspector and select element
      act(() => {
        result.current.toggleInspector()
        result.current.setSelectedElement(mockElement)
      })

      expect(result.current.isInspectorActive).toBe(true)
      expect(result.current.selectedElement).toEqual(mockElement)

      // Deactivate inspector - should clear selected element
      act(() => {
        result.current.toggleInspector()
      })

      expect(result.current.isInspectorActive).toBe(false)
      expect(result.current.selectedElement).toBe(null)
    })

    it('should not clear selected element when activating inspector', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })
      const mockElement = createMockElementInfo()

      // Set element while inspector is inactive
      act(() => {
        result.current.setSelectedElement(mockElement)
      })

      expect(result.current.selectedElement).toEqual(mockElement)

      // Activate inspector - should preserve selected element
      act(() => {
        result.current.toggleInspector()
      })

      expect(result.current.isInspectorActive).toBe(true)
      expect(result.current.selectedElement).toEqual(mockElement)
    })

    it('should update selected element', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })
      const mockElement = createMockElementInfo()

      act(() => {
        result.current.setSelectedElement(mockElement)
      })

      expect(result.current.selectedElement).toEqual(mockElement)

      act(() => {
        result.current.setSelectedElement(null)
      })

      expect(result.current.selectedElement).toBe(null)
    })
  })

  describe('Error and Loading State Management', () => {
    it('should update error state', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      act(() => {
        result.current.setError('Test error')
      })

      expect(result.current.error).toBe('Test error')
    })

    it('should set isLoaded to false when setting error', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      // First set loaded to true
      act(() => {
        result.current.setLoaded(true)
      })

      expect(result.current.isLoaded).toBe(true)

      // Setting error should set loaded to false
      act(() => {
        result.current.setError('Test error')
      })

      expect(result.current.error).toBe('Test error')
      expect(result.current.isLoaded).toBe(false)
    })

    it('should update loaded state', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      act(() => {
        result.current.setLoaded(true)
      })

      expect(result.current.isLoaded).toBe(true)
    })

    it('should clear error when setting loaded to true', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      // First set error
      act(() => {
        result.current.setError('Test error')
      })

      expect(result.current.error).toBe('Test error')

      // Setting loaded to true should clear error
      act(() => {
        result.current.setLoaded(true)
      })

      expect(result.current.isLoaded).toBe(true)
      expect(result.current.error).toBe(null)
    })

    it('should update connection state', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      act(() => {
        result.current.setConnected(true)
      })

      expect(result.current.isConnected).toBe(true)
    })
  })

  describe('Script Health Management', () => {
    it('should update script health status', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })
      const statuses: ScriptHealthStatus[] = [
        'not-injected',
        'injected-not-reachable',
        'injected-reachable',
      ]

      for (const status of statuses) {
        act(() => {
          result.current.setScriptHealthStatus(status)
        })

        expect(result.current.scriptHealthStatus).toBe(status)
      }
    })

    it('should update heartbeat and set status to injected-reachable', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })
      const beforeTimestamp = Date.now()

      act(() => {
        result.current.updateHeartbeat()
      })

      const afterTimestamp = Date.now()

      expect(result.current.lastHeartbeat).toBeGreaterThanOrEqual(beforeTimestamp)
      expect(result.current.lastHeartbeat).toBeLessThanOrEqual(afterTimestamp)
      expect(result.current.scriptHealthStatus).toBe('injected-reachable')
    })
  })

  describe('Reset Functionality', () => {
    it('should reset all state to initial values', () => {
      const { result } = renderHook(() => usePreview(), { wrapper })

      // Change all state values
      act(() => {
        result.current.setConfig({ url: 'http://localhost:4000' })
        result.current.setCurrentPath('/changed')
        result.current.setDisplayPath('/different')
        result.current.toggleInspector()
        result.current.setLoaded(true)
        result.current.setConnected(true)
        result.current.setError('Some error')
        result.current.setScriptHealthStatus('injected-reachable')
        result.current.setSelectedElement(
          createMockElementInfo({
            selector: '.test',
            displayText: 'Test',
            textContent: 'Test',
          }),
        )
        result.current.updateHeartbeat()
      })

      // Verify state was changed
      expect(result.current.config.url).toBe('http://localhost:4000')
      expect(result.current.currentPath).toBe('/changed')
      expect(result.current.isInspectorActive).toBe(true)

      // Reset
      act(() => {
        result.current.reset()
      })

      // Verify all state is back to initial values
      expect(result.current.config).toEqual({ url: 'http://localhost:3000' })
      expect(result.current.currentPath).toBe('/')
      expect(result.current.displayPath).toBe('/')
      expect(result.current.isInspectorActive).toBe(false)
      expect(result.current.isLoaded).toBe(false)
      expect(result.current.selectedElement).toBe(null)
      expect(result.current.error).toBe(null)
      expect(result.current.isConnected).toBe(false)
      expect(result.current.scriptHealthStatus).toBe('not-injected')
      expect(result.current.lastHeartbeat).toBe(null)
    })
  })

  describe('Error Boundaries', () => {
    it('should throw error when used outside provider', () => {
      // Mock console.error to avoid noise in test output
      const consoleSpy = vi.spyOn(console, 'error').mockImplementation(() => undefined)

      expect(() => {
        renderHook(() => usePreview())
      }).toThrow('usePreview must be used within PreviewProvider')

      consoleSpy.mockRestore()
    })
  })
})
