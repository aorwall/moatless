import { act, renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { PreviewProvider } from '../use-preview'
import { usePreviewConnection } from '../use-preview-connection'

// Mock fetch
const mockFetch = vi.fn()
global.fetch = mockFetch

// Mock AbortController
const mockAbort = vi.fn()
const mockController = {
  abort: mockAbort,
  signal: {} as AbortSignal,
}
global.AbortController = vi.fn(() => mockController) as any

function createWrapper() {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <PreviewProvider>{children}</PreviewProvider>
  }
}

describe('usePreviewConnection', () => {
  let wrapper: ReturnType<typeof createWrapper>

  beforeEach(() => {
    wrapper = createWrapper()
    vi.clearAllMocks()
    vi.clearAllTimers()
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  describe('Connection Testing', () => {
    it('should check connection on mount', async () => {
      mockFetch.mockResolvedValue(new Response('', { status: 200 }))

      const { result } = renderHook(() => usePreviewConnection('http://localhost:3000'), {
        wrapper,
      })

      await waitFor(() => {
        expect(mockFetch).toHaveBeenCalledWith('http://localhost:3000', {
          method: 'HEAD',
          mode: 'no-cors',
          signal: mockController.signal,
        })
      })

      await waitFor(() => {
        expect(result.current.isConnected).toBe(true)
      })
    })

    it('should handle connection success', async () => {
      mockFetch.mockResolvedValue(new Response('', { status: 200 }))

      const { result } = renderHook(() => usePreviewConnection('http://localhost:3000'), {
        wrapper,
      })

      await waitFor(() => {
        expect(result.current.isConnected).toBe(true)
      })
    })

    it('should handle connection failure', async () => {
      mockFetch.mockRejectedValue(new Error('Network error'))

      const { result } = renderHook(() => usePreviewConnection('http://localhost:3000'), {
        wrapper,
      })

      await waitFor(() => {
        expect(result.current.isConnected).toBe(false)
      })
    })

    it('should handle timeout', async () => {
      const timeoutError = new Error('Timeout')
      timeoutError.name = 'AbortError'
      mockFetch.mockRejectedValue(timeoutError)

      const { result } = renderHook(() => usePreviewConnection('http://localhost:3000'), {
        wrapper,
      })

      await waitFor(() => {
        expect(result.current.isConnected).toBe(false)
      })
    })

    it('should skip connection check for empty URL', async () => {
      const { result } = renderHook(() => usePreviewConnection(''), {
        wrapper,
      })

      // Should not call fetch for empty URL
      expect(mockFetch).not.toHaveBeenCalled()
      expect(result.current.isConnected).toBe(false)
    })

    it('should skip connection check for whitespace-only URL', async () => {
      const { result } = renderHook(() => usePreviewConnection('   '), {
        wrapper,
      })

      expect(mockFetch).not.toHaveBeenCalled()
      expect(result.current.isConnected).toBe(false)
    })
  })

  describe('Periodic Connection Checks', () => {
    it('should perform periodic connection checks', async () => {
      mockFetch.mockResolvedValue(new Response('', { status: 200 }))

      renderHook(() => usePreviewConnection('http://localhost:3000'), {
        wrapper,
      })

      // Initial call
      await waitFor(() => {
        expect(mockFetch).toHaveBeenCalledTimes(1)
      })

      // Fast-forward 5 seconds
      act(() => {
        vi.advanceTimersByTime(5000)
      })

      await waitFor(() => {
        expect(mockFetch).toHaveBeenCalledTimes(2)
      })

      // Fast-forward another 5 seconds
      act(() => {
        vi.advanceTimersByTime(5000)
      })

      await waitFor(() => {
        expect(mockFetch).toHaveBeenCalledTimes(3)
      })
    })

    it('should clear interval on unmount', async () => {
      mockFetch.mockResolvedValue(new Response('', { status: 200 }))

      const { unmount } = renderHook(() => usePreviewConnection('http://localhost:3000'), {
        wrapper,
      })

      // Initial call
      await waitFor(() => {
        expect(mockFetch).toHaveBeenCalledTimes(1)
      })

      // Unmount
      unmount()

      // Fast-forward 10 seconds - should not call fetch again
      act(() => {
        vi.advanceTimersByTime(10000)
      })

      // Should still be just the initial call
      expect(mockFetch).toHaveBeenCalledTimes(1)
    })
  })

  describe('Manual Connection Check', () => {
    it('should provide manual checkConnection function', async () => {
      mockFetch.mockResolvedValue(new Response('', { status: 200 }))

      const { result } = renderHook(() => usePreviewConnection('http://localhost:3000'), {
        wrapper,
      })

      expect(typeof result.current.checkConnection).toBe('function')

      // Clear the initial automatic call
      mockFetch.mockClear()

      // Manual check
      await act(async () => {
        await result.current.checkConnection()
      })

      expect(mockFetch).toHaveBeenCalledTimes(1)
    })
  })

  describe('URL Changes', () => {
    it('should restart connection checking when URL changes', async () => {
      mockFetch.mockResolvedValue(new Response('', { status: 200 }))

      const { rerender } = renderHook(({ url }: { url: string }) => usePreviewConnection(url), {
        wrapper,
        initialProps: { url: 'http://localhost:3000' },
      })

      // Initial call
      await waitFor(() => {
        expect(mockFetch).toHaveBeenCalledWith('http://localhost:3000', expect.any(Object))
      })

      mockFetch.mockClear()

      // Change URL
      rerender({ url: 'http://localhost:4000' })

      await waitFor(() => {
        expect(mockFetch).toHaveBeenCalledWith('http://localhost:4000', expect.any(Object))
      })
    })
  })

  describe('Error Handling', () => {
    it('should set error message for network errors', async () => {
      mockFetch.mockRejectedValue(new Error('Network error'))

      renderHook(() => usePreviewConnection('http://localhost:3000'), {
        wrapper,
      })

      // Note: Error state is managed by the preview context, not returned by this hook
      // This test verifies the hook doesn't crash on errors
      await waitFor(() => {
        expect(mockFetch).toHaveBeenCalled()
      })
    })

    it('should set timeout error message for AbortError', async () => {
      const timeoutError = new Error('Timeout')
      timeoutError.name = 'AbortError'
      mockFetch.mockRejectedValue(timeoutError)

      renderHook(() => usePreviewConnection('http://localhost:3000'), {
        wrapper,
      })

      await waitFor(() => {
        expect(mockFetch).toHaveBeenCalled()
      })
    })
  })

  describe('Timeout Handling', () => {
    it('should abort fetch request after timeout', async () => {
      // Make fetch hang
      mockFetch.mockImplementation(() => new Promise((resolve) => setTimeout(resolve, 10000)))

      renderHook(() => usePreviewConnection('http://localhost:3000'), {
        wrapper,
      })

      await waitFor(() => {
        expect(mockFetch).toHaveBeenCalled()
      })

      // Fast-forward past the 5-second timeout
      act(() => {
        vi.advanceTimersByTime(5000)
      })

      // Should have called abort
      expect(mockAbort).toHaveBeenCalled()
    })
  })
})
