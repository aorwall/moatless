import type { Message } from '@moatless/types'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderHook, waitFor } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { setupServer } from 'msw/node'
import type React from 'react'
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useTaskMessages } from '../use-tasks'

// Mock data
const mockMessages: Message[] = [
  {
    id: 'msg-1',
    role: 'user',
    content: 'Hello, can you help me with this task?',
    timestamp: new Date('2024-01-01T10:00:00Z'),
  },
  {
    id: 'msg-2',
    role: 'assistant',
    content: [
      {
        type: 'text',
        text: "I'll help you with that. Let me run some commands first.",
      },
      {
        type: 'tool_use',
        tool_name: 'Bash',
        tool_input: { command: 'ls -la' },
        tool_use_id: 'tool-1',
        tool_result: {
          content:
            'total 8\ndrwxr-xr-x 3 user user 4096 Jan  1 10:00 .\ndrwxr-xr-x 3 user user 4096 Jan  1 09:59 ..',
        },
      },
    ],
    timestamp: new Date('2024-01-01T10:01:00Z'),
  },
  {
    id: 'msg-3',
    role: 'user',
    content: "Thanks! That's helpful.",
    timestamp: new Date('2024-01-01T10:02:00Z'),
  },
]

// Setup MSW server
const server = setupServer(
  // Success response
  http.get('/api/v1/tasks/:taskId/messages', ({ params }) => {
    const { taskId } = params as { taskId: string }

    if (taskId === 'valid-task') {
      return HttpResponse.json({
        success: true,
        data: mockMessages,
      })
    }

    if (taskId === 'empty-task') {
      return HttpResponse.json({
        success: true,
        data: [],
      })
    }

    if (taskId === 'error-task') {
      return HttpResponse.json(
        {
          success: false,
          error: 'Task not found',
        },
        { status: 404 },
      )
    }

    if (taskId === 'server-error-task') {
      return HttpResponse.json(
        {
          success: false,
          error: 'Internal server error',
        },
        { status: 500 },
      )
    }

    return HttpResponse.json(
      {
        success: false,
        error: 'Task not found',
      },
      { status: 404 },
    )
  }),
)

describe('useTaskMessages Hook - Integration Tests', () => {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
      },
    },
  })

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  )

  beforeEach(() => {
    server.listen({ onUnhandledRequest: 'error' })
    vi.clearAllMocks()
  })

  afterEach(() => {
    server.resetHandlers()
  })

  afterAll(() => {
    server.close()
  })

  describe('Successful data fetching', () => {
    it('fetches and returns task messages', async () => {
      const { result } = renderHook(() => useTaskMessages('valid-task'), { wrapper })

      // Initially loading
      expect(result.current.isLoading).toBe(true)
      expect(result.current.data).toBeUndefined()
      expect(result.current.error).toBeNull()

      // Wait for data to load
      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      // Check final state
      expect(result.current.data).toHaveLength(3)
      expect(result.current.data).toEqual(mockMessages)
      expect(result.current.error).toBeNull()
      expect(result.current.isSuccess).toBe(true)
    })

    it('handles empty messages array', async () => {
      const { result } = renderHook(() => useTaskMessages('empty-task'), { wrapper })

      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      expect(result.current.data).toHaveLength(0)
      expect(result.current.data).toEqual([])
      expect(result.current.isSuccess).toBe(true)
    })

    it('includes proper React Query metadata', async () => {
      const { result } = renderHook(() => useTaskMessages('valid-task'), { wrapper })

      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      // Check React Query specific properties
      expect(result.current.isFetching).toBe(false)
      expect(result.current.isSuccess).toBe(true)
      expect(result.current.isError).toBe(false)
      expect(result.current.dataUpdatedAt).toBeGreaterThan(0)
      expect(result.current.status).toBe('success')
    })
  })

  describe('Error handling', () => {
    it('handles 404 task not found error', async () => {
      const { result } = renderHook(() => useTaskMessages('error-task'), { wrapper })

      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      expect(result.current.isError).toBe(true)
      expect(result.current.error).toBeTruthy()
      expect(result.current.data).toBeUndefined()
      expect(result.current.status).toBe('error')
    })

    it('handles 500 server error', async () => {
      const { result } = renderHook(() => useTaskMessages('server-error-task'), { wrapper })

      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      expect(result.current.isError).toBe(true)
      expect(result.current.error).toBeTruthy()
      expect(result.current.data).toBeUndefined()
    })

    it('handles network errors', async () => {
      server.use(
        http.get('/api/v1/tasks/:taskId/messages', () => {
          return HttpResponse.error()
        }),
      )

      const { result } = renderHook(() => useTaskMessages('network-error-task'), { wrapper })

      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      expect(result.current.isError).toBe(true)
      expect(result.current.error).toBeTruthy()
      expect(result.current.data).toBeUndefined()
    })
  })

  describe('Caching behavior', () => {
    it('caches data between hook instances', async () => {
      // First hook instance
      const { result: result1 } = renderHook(() => useTaskMessages('valid-task'), { wrapper })

      await waitFor(() => {
        expect(result1.current.isLoading).toBe(false)
      })

      expect(result1.current.data).toHaveLength(3)

      // Second hook instance should use cached data
      const { result: result2 } = renderHook(() => useTaskMessages('valid-task'), { wrapper })

      // Should immediately have data from cache
      expect(result2.current.data).toHaveLength(3)
      expect(result2.current.isLoading).toBe(false)
      expect(result2.current.isSuccess).toBe(true)
    })

    it('handles different task IDs separately', async () => {
      const { result: result1 } = renderHook(() => useTaskMessages('valid-task'), { wrapper })

      const { result: result2 } = renderHook(() => useTaskMessages('empty-task'), { wrapper })

      await waitFor(() => {
        expect(result1.current.isLoading).toBe(false)
        expect(result2.current.isLoading).toBe(false)
      })

      expect(result1.current.data).toHaveLength(3)
      expect(result2.current.data).toHaveLength(0)
    })
  })

  describe('Loading states', () => {
    it('shows initial loading state', () => {
      const { result } = renderHook(() => useTaskMessages('valid-task'), { wrapper })

      expect(result.current.isLoading).toBe(true)
      expect(result.current.isFetching).toBe(true)
      expect(result.current.data).toBeUndefined()
      expect(result.current.status).toBe('pending')
    })

    it('transitions from loading to success', async () => {
      const { result } = renderHook(() => useTaskMessages('valid-task'), { wrapper })

      // Initial state
      expect(result.current.status).toBe('pending')
      expect(result.current.isLoading).toBe(true)

      // Final state
      await waitFor(() => {
        expect(result.current.status).toBe('success')
      })

      expect(result.current.isLoading).toBe(false)
      expect(result.current.isSuccess).toBe(true)
    })

    it('transitions from loading to error', async () => {
      const { result } = renderHook(() => useTaskMessages('error-task'), { wrapper })

      // Initial state
      expect(result.current.status).toBe('pending')
      expect(result.current.isLoading).toBe(true)

      // Final state
      await waitFor(() => {
        expect(result.current.status).toBe('error')
      })

      expect(result.current.isLoading).toBe(false)
      expect(result.current.isError).toBe(true)
    })
  })

  describe('Refetch behavior', () => {
    it('supports manual refetch', async () => {
      const { result } = renderHook(() => useTaskMessages('valid-task'), { wrapper })

      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      expect(result.current.data).toHaveLength(3)

      // Manually refetch
      await result.current.refetch()

      expect(result.current.data).toHaveLength(3)
      expect(result.current.isSuccess).toBe(true)
    })

    it('handles refetch errors gracefully', async () => {
      const { result } = renderHook(() => useTaskMessages('valid-task'), { wrapper })

      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      // Change server to return error for refetch
      server.use(
        http.get('/api/v1/tasks/valid-task/messages', () => {
          return HttpResponse.json({ success: false, error: 'Server error' }, { status: 500 })
        }),
      )

      // Refetch should handle error
      await result.current.refetch()

      expect(result.current.isError).toBe(true)
    })
  })

  describe('Type safety', () => {
    it('returns properly typed message data', async () => {
      const { result } = renderHook(() => useTaskMessages('valid-task'), { wrapper })

      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      const messages = result.current.data!

      // Type assertions - these should compile without errors
      expect(messages[0].id).toBe('msg-1')
      expect(messages[0].role).toBe('user')
      expect(messages[0].content).toBe('Hello, can you help me with this task?')
      expect(messages[0].timestamp).toEqual(new Date('2024-01-01T10:00:00Z'))

      // Check assistant message with tool use
      expect(messages[1].role).toBe('assistant')
      expect(Array.isArray(messages[1].content)).toBe(true)
      const content = messages[1].content as any[]
      expect(content[0].type).toBe('text')
      expect(content[1].type).toBe('tool_use')
      expect(content[1].tool_name).toBe('Bash')
      expect(content[1].tool_input).toEqual({ command: 'ls -la' })
      expect(content[1].tool_result).toBeTruthy()
    })
  })

  describe('Edge cases', () => {
    it('handles undefined taskId gracefully', async () => {
      const { result } = renderHook(
        // @ts-expect-error Testing runtime behavior
        () => useTaskMessages(undefined),
        { wrapper },
      )

      // Hook should be disabled when taskId is undefined
      expect(result.current.data).toBeUndefined()
      expect(result.current.isLoading).toBe(false)
      expect(result.current.isError).toBe(false)
    })

    it('handles empty string taskId', async () => {
      const { result } = renderHook(() => useTaskMessages(''), { wrapper })

      // Should not make request with empty taskId
      expect(result.current.data).toBeUndefined()
      expect(result.current.isLoading).toBe(false)
    })

    it('handles very long taskId', async () => {
      const longTaskId = `task-${'a'.repeat(1000)}`

      server.use(
        http.get(`/api/v1/tasks/${longTaskId}/messages`, () => {
          return HttpResponse.json({
            success: true,
            data: [],
          })
        }),
      )

      const { result } = renderHook(() => useTaskMessages(longTaskId), { wrapper })

      await waitFor(() => {
        expect(result.current.isLoading).toBe(false)
      })

      expect(result.current.isSuccess).toBe(true)
      expect(result.current.data).toEqual([])
    })
  })
})
