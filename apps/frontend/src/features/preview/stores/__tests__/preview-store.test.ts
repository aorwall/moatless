import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { usePreviewStore } from '../preview-store'

describe('PreviewStore - Script Health Functionality', () => {
  beforeEach(() => {
    // Reset store state before each test
    act(() => {
      usePreviewStore.getState().reset()
    })
  })

  describe('Initial State', () => {
    it('initializes with not-injected status', () => {
      const { result } = renderHook(() => usePreviewStore())

      expect(result.current.scriptHealthStatus).toBe('not-injected')
      expect(result.current.lastHeartbeat).toBeNull()
    })
  })

  describe('setScriptHealthStatus', () => {
    it('updates script health status', () => {
      const { result } = renderHook(() => usePreviewStore())

      act(() => {
        result.current.setScriptHealthStatus('injected-reachable')
      })

      expect(result.current.scriptHealthStatus).toBe('injected-reachable')
    })

    it('allows transitioning between all status states', () => {
      const { result } = renderHook(() => usePreviewStore())

      // Start with not-injected
      expect(result.current.scriptHealthStatus).toBe('not-injected')

      // Transition to injected-reachable
      act(() => {
        result.current.setScriptHealthStatus('injected-reachable')
      })
      expect(result.current.scriptHealthStatus).toBe('injected-reachable')

      // Transition to injected-not-reachable
      act(() => {
        result.current.setScriptHealthStatus('injected-not-reachable')
      })
      expect(result.current.scriptHealthStatus).toBe('injected-not-reachable')

      // Back to not-injected
      act(() => {
        result.current.setScriptHealthStatus('not-injected')
      })
      expect(result.current.scriptHealthStatus).toBe('not-injected')
    })
  })

  describe('updateHeartbeat', () => {
    it('updates lastHeartbeat timestamp', () => {
      const { result } = renderHook(() => usePreviewStore())
      const beforeTime = Date.now()

      act(() => {
        result.current.updateHeartbeat()
      })

      const afterTime = Date.now()

      expect(result.current.lastHeartbeat).toBeGreaterThanOrEqual(beforeTime)
      expect(result.current.lastHeartbeat).toBeLessThanOrEqual(afterTime)
    })

    it('sets script health status to injected-reachable', () => {
      const { result } = renderHook(() => usePreviewStore())

      // Start with different status
      act(() => {
        result.current.setScriptHealthStatus('injected-not-reachable')
      })

      act(() => {
        result.current.updateHeartbeat()
      })

      expect(result.current.scriptHealthStatus).toBe('injected-reachable')
    })

    it('updates heartbeat multiple times with increasing timestamps', () => {
      const { result } = renderHook(() => usePreviewStore())
      const timestamps: number[] = []

      // Update heartbeat 3 times with small delays
      for (let i = 0; i < 3; i++) {
        act(() => {
          result.current.updateHeartbeat()
        })
        timestamps.push(result.current.lastHeartbeat!)

        // Small delay to ensure different timestamps
        const start = Date.now()
        while (Date.now() - start < 10) {
          // Busy wait
        }
      }

      // Verify timestamps are increasing
      expect(timestamps[1]).toBeGreaterThan(timestamps[0])
      expect(timestamps[2]).toBeGreaterThan(timestamps[1])
    })
  })

  describe('reset', () => {
    it('resets script health status to initial state', () => {
      const { result } = renderHook(() => usePreviewStore())

      // Modify state
      act(() => {
        result.current.setScriptHealthStatus('injected-reachable')
        result.current.updateHeartbeat()
      })

      // Verify state was changed
      expect(result.current.scriptHealthStatus).toBe('injected-reachable')
      expect(result.current.lastHeartbeat).not.toBeNull()

      // Reset
      act(() => {
        result.current.reset()
      })

      // Verify reset to initial state
      expect(result.current.scriptHealthStatus).toBe('not-injected')
      expect(result.current.lastHeartbeat).toBeNull()
    })
  })

  describe('Integration with other store actions', () => {
    it('maintains script health status when other state changes', () => {
      const { result } = renderHook(() => usePreviewStore())

      // Set script health status
      act(() => {
        result.current.setScriptHealthStatus('injected-reachable')
        result.current.updateHeartbeat()
      })

      const heartbeatTime = result.current.lastHeartbeat

      // Change other state
      act(() => {
        result.current.setLoaded(true)
        result.current.setCurrentPath('/test')
        result.current.toggleInspector()
      })

      // Verify script health state is maintained
      expect(result.current.scriptHealthStatus).toBe('injected-reachable')
      expect(result.current.lastHeartbeat).toBe(heartbeatTime)
    })

    it('resets script health when setLoaded is called with false', () => {
      const { result } = renderHook(() => usePreviewStore())

      // Set up initial state
      act(() => {
        result.current.setScriptHealthStatus('injected-reachable')
        result.current.updateHeartbeat()
        result.current.setLoaded(true)
      })

      // When iframe is unloaded, script health should not be affected by setLoaded
      act(() => {
        result.current.setLoaded(false)
      })

      // Script health status should remain unchanged
      expect(result.current.scriptHealthStatus).toBe('injected-reachable')
      expect(result.current.lastHeartbeat).not.toBeNull()
    })
  })
})
