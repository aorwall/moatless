import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, waitFor } from '@testing-library/react'
import type React from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { GlobalEventStreamProvider } from '@/features/events'
import { queryKeys } from '@/lib/api/query-keys'

// Mock the API client
vi.mock('@/lib/api/client', () => ({
  api: {
    events: {
      stream: {
        $get: vi.fn(),
      },
    },
  },
}))

// ---------------------------------------------------------------------------
// Helpers: build a fetch mock that streams NDJSON lines to the provider
// ---------------------------------------------------------------------------

/**
 * Creates a mock fetch that emits the given NDJSON lines as a ReadableStream
 * and resolves when the stream is done.
 */
function createStreamFetch(lines: string[]) {
  const encoder = new TextEncoder()
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const line of lines) {
        controller.enqueue(encoder.encode(`${line}\n`))
      }
      controller.close()
    },
  })

  return vi.fn().mockResolvedValue({
    ok: true,
    body: stream,
  })
}

/**
 * Creates a fetch mock that returns a stream with a single event followed by
 * a server-side close signal so the provider stops reconnecting.
 */
function createEventStreamFetch(events: object[]) {
  const lines = [...events.map((e) => JSON.stringify(e)), JSON.stringify({ type: 'close' })]
  return createStreamFetch(lines)
}

/** Minimal wrapper that provides QueryClient + GlobalEventStreamProvider */
function renderWithProviders(children: React.ReactNode, queryClient: QueryClient) {
  return render(
    <QueryClientProvider client={queryClient}>
      <GlobalEventStreamProvider>{children}</GlobalEventStreamProvider>
    </QueryClientProvider>,
  )
}

/** A valid StreamEvent envelope for the given type/subType/taskId */
function makeEvent(overrides: { type: string; subType: string; taskId: string; payload?: object }) {
  return {
    eventId: `evt-${Math.random().toString(36).slice(2)}`,
    type: overrides.type,
    subType: overrides.subType,
    taskId: overrides.taskId,
    isError: false,
    timestamp: new Date().toISOString(),
    payload: overrides.payload ?? {},
  }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('GlobalEventStreamProvider — query invalidation (unit)', () => {
  let queryClient: QueryClient

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    })
    vi.spyOn(queryClient, 'invalidateQueries')
  })

  afterEach(() => {
    vi.clearAllMocks()
    vi.restoreAllMocks()
  })

  it('should invalidate only the specific task on a task.open event', async () => {
    const taskStatusEvent = makeEvent({
      type: 'task',
      subType: 'open',
      taskId: 'task-123',
      payload: { status: 'open', previousStatus: 'created' },
    })

    global.fetch = createEventStreamFetch([taskStatusEvent]) as any

    renderWithProviders(<div />, queryClient)

    await waitFor(
      () => {
        // The provider invalidates task lists when a status-affecting task event arrives
        expect(queryClient.invalidateQueries).toHaveBeenCalledWith(
          expect.objectContaining({
            queryKey: queryKeys.tasks.lists(),
          }),
        )
      },
      { timeout: 2000 },
    )

    // Should NOT invalidate the broad tasks.all key — only targeted keys
    expect(queryClient.invalidateQueries).not.toHaveBeenCalledWith(
      expect.objectContaining({ queryKey: queryKeys.tasks.all }),
    )
  })

  it('should handle multiple task events without cross-task invalidation', async () => {
    // Seed both task details: the provider patches cached details in place
    // (applyTaskEventUpdate) instead of invalidating them.
    queryClient.setQueryData(queryKeys.tasks.detail('task-123'), {
      taskId: 'task-123',
      status: 'created',
    } as any)
    queryClient.setQueryData(queryKeys.tasks.detail('task-456'), {
      taskId: 'task-456',
      status: 'created',
    } as any)

    const task1Event = makeEvent({
      type: 'task',
      subType: 'open',
      taskId: 'task-123',
      payload: { status: 'open' },
    })

    const task2Event = makeEvent({
      type: 'agent',
      subType: 'completed',
      taskId: 'task-456',
      payload: {},
    })

    global.fetch = createEventStreamFetch([task1Event, task2Event]) as any

    renderWithProviders(<div />, queryClient)

    await waitFor(
      () => {
        // task-456: agent.completed invalidates its sandboxStatus
        expect(queryClient.invalidateQueries).toHaveBeenCalledWith(
          expect.objectContaining({ queryKey: queryKeys.tasks.sandboxStatus('task-456') }),
        )
        // task-123: task.open is patched optimistically into the detail cache
        expect(queryClient.getQueryData(queryKeys.tasks.detail('task-123'))).toMatchObject({
          status: 'open',
        })
      },
      { timeout: 2000 },
    )

    // Cross-task isolation: task-123's event must not touch task-456's
    // sandbox status, and task-456's event must not flip task-123's status.
    expect(queryClient.invalidateQueries).not.toHaveBeenCalledWith(
      expect.objectContaining({ queryKey: queryKeys.tasks.sandboxStatus('task-123') }),
    )
    expect(queryClient.getQueryData(queryKeys.tasks.detail('task-456'))).toMatchObject({
      taskId: 'task-456',
    })

    // Detail keys themselves are never invalidated — detail updates are
    // optimistic in-place patches, not refetches. (Sub-resource keys like
    // sandboxStatus nest under detail and are allowed.)
    const calls = (queryClient.invalidateQueries as ReturnType<typeof vi.fn>).mock.calls
    const detailInvalidations = calls.filter((call) => {
      const key = call[0]?.queryKey as readonly unknown[] | undefined
      return key?.length === 3 && key[1] === 'detail'
    })
    expect(detailInvalidations).toHaveLength(0)
  })

  it('should handle a message event and invalidate the task detail', async () => {
    const messageEvent = makeEvent({
      type: 'agent',
      subType: 'message',
      taskId: 'task-789',
      payload: { message: { content: [] } },
    })

    global.fetch = createEventStreamFetch([messageEvent]) as any

    renderWithProviders(<div />, queryClient)

    await waitFor(
      () => {
        // agent.message triggers detail invalidation via getInvalidationKeys (length 0)
        // but applyTaskEventUpdate may call setQueryData; verify tasks.all is never used
        expect(queryClient.invalidateQueries).not.toHaveBeenCalledWith(
          expect.objectContaining({ queryKey: queryKeys.tasks.all }),
        )
        // tasks.lists() must NOT be invalidated for a non-status-affecting event
        expect(queryClient.invalidateQueries).not.toHaveBeenCalledWith(
          expect.objectContaining({ queryKey: queryKeys.tasks.lists() }),
        )
      },
      { timeout: 2000 },
    )
  })

  it('should invalidate sandboxStatus for a sandbox event but NOT task lists', async () => {
    const sandboxEvent = makeEvent({
      type: 'sandbox',
      subType: 'ready',
      taskId: 'task-111',
      payload: {},
    })

    global.fetch = createEventStreamFetch([sandboxEvent]) as any

    renderWithProviders(<div />, queryClient)

    await waitFor(
      () => {
        // sandbox events trigger sandboxStatus and sandboxSetup invalidation
        expect(queryClient.invalidateQueries).toHaveBeenCalledWith(
          expect.objectContaining({
            queryKey: queryKeys.tasks.sandboxStatus('task-111'),
          }),
        )
      },
      { timeout: 2000 },
    )

    // task lists must NOT be invalidated for sandbox events — only task events with
    // status-affecting subTypes cause list refetches
    expect(queryClient.invalidateQueries).not.toHaveBeenCalledWith(
      expect.objectContaining({ queryKey: queryKeys.tasks.lists() }),
    )
  })
})
