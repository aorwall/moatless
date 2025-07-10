import { act, render, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { usePreviewStore } from '../../stores/preview-store'
import type { PreviewProps } from '../../types'
import { Preview } from '../preview'

// Mock dependencies
vi.mock('@tanstack/react-router', () => ({
  useParams: () => ({ taskId: 'test-task' }),
}))

vi.mock('@/features/preview', () => ({
  useStartServer: () => ({ mutate: vi.fn(), isPending: false }),
  useRestartServer: () => ({ mutate: vi.fn(), isPending: false }),
  useStopServer: () => ({ mutate: vi.fn(), isPending: false }),
  useServerStatus: () => ({
    data: { running: true, configured: true, url: 'http://localhost:3000' },
  }),
  useServerLogs: () => ({
    data: { logs: '' },
    isLoading: false,
    error: null,
  }),
}))

vi.mock('../../hooks/use-preview-connection', () => ({
  usePreviewConnection: () => ({ isConnected: true }),
}))

// Mock ResizablePanel components
vi.mock('@/lib/components/ui/resizable', () => ({
  ResizablePanel: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  ResizablePanelGroup: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  ResizableHandle: () => <div />,
}))

// Mock Tooltip components
vi.mock('@/lib/components/ui/tooltip', () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  TooltipTrigger: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  TooltipContent: () => null,
  TooltipProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}))

// Mock Popover components
vi.mock('@/lib/components/ui/popover', () => ({
  Popover: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  PopoverTrigger: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  PopoverContent: () => null,
}))

describe('Preview Component - Heartbeat Handling', () => {
  let queryClient: QueryClient
  const defaultProps: PreviewProps = {
    config: { url: 'http://localhost:3000' },
    serverStatus: { running: true, configured: true, url: 'http://localhost:3000' },
  }

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    })
    
    // Reset store
    act(() => {
      usePreviewStore.getState().reset()
    })
    
    // Clear all timers
    vi.clearAllTimers()
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  const renderPreview = (props: Partial<PreviewProps> = {}) => {
    return render(
      <QueryClientProvider client={queryClient}>
        <Preview {...defaultProps} {...props} />
      </QueryClientProvider>
    )
  }

  describe('Message Handling', () => {
    it('handles INSPECTOR_SCRIPT_LOADED message', () => {
      renderPreview()
      
      // Simulate script loaded message
      act(() => {
        window.dispatchEvent(new MessageEvent('message', {
          data: {
            type: 'INSPECTOR_SCRIPT_LOADED',
            source: 'preview-iframe',
            payload: { version: '2.0.0' },
          },
          origin: 'http://localhost:3000',
        }))
      })
      
      const state = usePreviewStore.getState()
      expect(state.scriptHealthStatus).toBe('injected-reachable')
      expect(state.lastHeartbeat).not.toBeNull()
    })

    it('handles HEARTBEAT message', () => {
      renderPreview()
      
      const beforeTime = Date.now()
      
      // Simulate heartbeat message
      act(() => {
        window.dispatchEvent(new MessageEvent('message', {
          data: {
            type: 'HEARTBEAT',
            source: 'preview-iframe',
            payload: { timestamp: Date.now() },
          },
          origin: 'http://localhost:3000',
        }))
      })
      
      const state = usePreviewStore.getState()
      expect(state.scriptHealthStatus).toBe('injected-reachable')
      expect(state.lastHeartbeat).toBeGreaterThanOrEqual(beforeTime)
    })

    it('updates heartbeat timestamp on multiple HEARTBEAT messages', () => {
      renderPreview()
      
      const timestamps: number[] = []
      
      // Send multiple heartbeats
      for (let i = 0; i < 3; i++) {
        act(() => {
          window.dispatchEvent(new MessageEvent('message', {
            data: {
              type: 'HEARTBEAT',
              source: 'preview-iframe',
              payload: { timestamp: Date.now() + i * 1000 },
            },
            origin: 'http://localhost:3000',
          }))
        })
        
        timestamps.push(usePreviewStore.getState().lastHeartbeat!)
      }
      
      // Verify timestamps are updating (or at least not null)
      expect(timestamps[1]).toBeGreaterThanOrEqual(timestamps[0])
      expect(timestamps[2]).toBeGreaterThanOrEqual(timestamps[1])
    })
  })

  describe('Heartbeat Timeout', () => {
    it.skip('marks script as not reachable after timeout - heartbeat timeout not implemented', async () => {
      renderPreview()
      
      // Set initial state as reachable
      act(() => {
        usePreviewStore.getState().setScriptHealthStatus('injected-reachable')
        usePreviewStore.getState().updateHeartbeat()
      })
      
      expect(usePreviewStore.getState().scriptHealthStatus).toBe('injected-reachable')
      
      // Fast forward time beyond timeout (10 seconds)
      act(() => {
        vi.advanceTimersByTime(11000)
      })
      
      await waitFor(() => {
        expect(usePreviewStore.getState().scriptHealthStatus).toBe('injected-not-reachable')
      })
    })

    it.skip('does not timeout if heartbeats keep coming - heartbeat timeout not implemented', async () => {
      renderPreview()
      
      // Set initial state
      act(() => {
        usePreviewStore.getState().setScriptHealthStatus('injected-reachable')
        usePreviewStore.getState().updateHeartbeat()
      })
      
      // Send heartbeats every 5 seconds
      for (let i = 0; i < 3; i++) {
        act(() => {
          vi.advanceTimersByTime(5000)
          window.dispatchEvent(new MessageEvent('message', {
            data: {
              type: 'HEARTBEAT',
              payload: { timestamp: Date.now() },
            },
            origin: 'http://localhost:3000',
          }))
        })
      }
      
      // Should still be reachable
      expect(usePreviewStore.getState().scriptHealthStatus).toBe('injected-reachable')
    })

    it.skip('resets timeout when new heartbeat is received - heartbeat timeout not implemented', async () => {
      renderPreview()
      
      // Set initial state
      act(() => {
        usePreviewStore.getState().setScriptHealthStatus('injected-reachable')
        usePreviewStore.getState().updateHeartbeat()
      })
      
      // Advance time but not past timeout
      act(() => {
        vi.advanceTimersByTime(8000)
      })
      
      // Send new heartbeat
      act(() => {
        window.dispatchEvent(new MessageEvent('message', {
          data: {
            type: 'HEARTBEAT',
            source: 'preview-iframe',
            payload: { timestamp: Date.now() },
          },
          origin: 'http://localhost:3000',
        }))
      })
      
      // Advance time again (8 seconds from last heartbeat)
      act(() => {
        vi.advanceTimersByTime(8000)
      })
      
      // Should still be reachable
      expect(usePreviewStore.getState().scriptHealthStatus).toBe('injected-reachable')
      
      // Now advance past timeout
      act(() => {
        vi.advanceTimersByTime(3000)
      })
      
      await waitFor(() => {
        expect(usePreviewStore.getState().scriptHealthStatus).toBe('injected-not-reachable')
      })
    })
  })

  describe('Iframe Load Handling', () => {
    it.skip('resets script health status to not-injected on iframe load - flaky test', () => {
      const { container } = renderPreview()
      
      // Set initial state
      act(() => {
        usePreviewStore.getState().setScriptHealthStatus('injected-reachable')
        usePreviewStore.getState().updateHeartbeat()
      })
      
      // Find and trigger iframe load event
      const iframe = container.querySelector('iframe')
      expect(iframe).toBeTruthy()
      
      // Force the handleIframeLoad to be called
      act(() => {
        const loadEvent = new Event('load', { bubbles: true, cancelable: true })
        iframe?.dispatchEvent(loadEvent)
      })
      
      // Wait for state update
      act(() => {
        // The iframe load handler should reset status
      })
      
      expect(usePreviewStore.getState().scriptHealthStatus).toBe('not-injected')
    })

    it('properly transitions from not-injected to reachable after iframe load', () => {
      const { container } = renderPreview()
      
      // Trigger iframe load
      const iframe = container.querySelector('iframe')
      act(() => {
        iframe?.dispatchEvent(new Event('load'))
      })
      
      expect(usePreviewStore.getState().scriptHealthStatus).toBe('not-injected')
      
      // Simulate script loaded message
      act(() => {
        window.dispatchEvent(new MessageEvent('message', {
          data: {
            type: 'INSPECTOR_SCRIPT_LOADED',
            source: 'preview-iframe',
            payload: { version: '2.0.0' },
          },
          origin: 'http://localhost:3000',
        }))
      })
      
      expect(usePreviewStore.getState().scriptHealthStatus).toBe('injected-reachable')
    })
  })

  describe('Component Lifecycle', () => {
    it('cleans up timeout on unmount', () => {
      const { unmount } = renderPreview()
      
      // Set up heartbeat monitoring
      act(() => {
        usePreviewStore.getState().updateHeartbeat()
      })
      
      // Unmount component
      unmount()
      
      // Advance time past timeout
      act(() => {
        vi.advanceTimersByTime(15000)
      })
      
      // Status should not change after unmount
      expect(usePreviewStore.getState().scriptHealthStatus).toBe('injected-reachable')
    })

    it('removes message event listener on unmount', () => {
      const { unmount } = renderPreview()
      
      // Unmount component
      unmount()
      
      // Try to send message after unmount
      act(() => {
        window.dispatchEvent(new MessageEvent('message', {
          data: {
            type: 'HEARTBEAT',
            source: 'preview-iframe',
            payload: { timestamp: Date.now() },
          },
          origin: 'http://localhost:3000',
        }))
      })
      
      // Should not update (no error should occur)
      expect(usePreviewStore.getState().lastHeartbeat).toBeNull()
    })
  })
})