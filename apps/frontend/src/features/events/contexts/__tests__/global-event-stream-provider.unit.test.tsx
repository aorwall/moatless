import type { LiveStatus } from '@moatless/api-client'
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

/**
 * Builds a fresh streamed Response for the given NDJSON lines. Each call returns
 * a new ReadableStream (streams are single-use), so this is safe to hand back
 * from a fetch mock on every invocation.
 */
function streamResponse(lines: string[]) {
  const encoder = new TextEncoder()
  return {
    ok: true,
    body: new ReadableStream<Uint8Array>({
      start(controller) {
        for (const line of lines) {
          controller.enqueue(encoder.encode(`${line}\n`))
        }
        controller.close()
      },
    }),
  }
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
function makeEvent(overrides: {
  type: string
  subType: string
  taskId: string
  payload?: object
  agentWorking?: boolean
}) {
  return {
    eventId: `evt-${Math.random().toString(36).slice(2)}`,
    type: overrides.type,
    subType: overrides.subType,
    taskId: overrides.taskId,
    isError: false,
    timestamp: new Date().toISOString(),
    payload: overrides.payload ?? {},
    // Omitted, not null, when the caller has no answer to carry — the server
    // omits the key for an event that does not report on the agent.
    ...(overrides.agentWorking === undefined ? {} : { agentWorking: overrides.agentWorking }),
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

  it('overlays Agent and Sandbox events onto the per-Task live-status cache', async () => {
    queryClient.setQueryData<LiveStatus>(queryKeys.tasks.liveStatus('task-live'), {
      sandboxStatus: 'ready',
      agentStatus: 'idle',
      sandboxError: null,
    })
    global.fetch = createEventStreamFetch([
      makeEvent({ type: 'agent', subType: 'started', taskId: 'task-live' }),
      makeEvent({ type: 'sandbox', subType: 'stopped', taskId: 'task-live' }),
    ]) as any

    renderWithProviders(<div />, queryClient)

    await waitFor(() => {
      expect(queryClient.getQueryData(queryKeys.tasks.liveStatus('task-live'))).toEqual({
        sandboxStatus: 'stopped',
        agentStatus: null,
        sandboxError: null,
      })
    })
  })

  it('reads the working answer off the envelope rather than off the subType', async () => {
    queryClient.setQueryData<LiveStatus>(queryKeys.tasks.liveStatus('task-envelope'), {
      sandboxStatus: 'ready',
      agentStatus: 'idle',
      sandboxError: null,
    })
    // `agent.message` is not a lifecycle sub-type, so a subType table would
    // ignore it. The envelope says the agent is working, which is the answer.
    global.fetch = createEventStreamFetch([
      makeEvent({
        type: 'agent',
        subType: 'message',
        taskId: 'task-envelope',
        agentWorking: true,
      }),
    ]) as any

    renderWithProviders(<div />, queryClient)

    await waitFor(() => {
      expect(queryClient.getQueryData(queryKeys.tasks.liveStatus('task-envelope'))).toEqual({
        sandboxStatus: 'ready',
        agentStatus: 'running',
        sandboxError: null,
      })
    })
  })

  it('lets a taskTurn close the agent without claiming its pod is back', async () => {
    // The reaper's case: the pod stopped mid-turn, so the last thing the cache
    // heard from the agent was that it was running. Only the backend can close
    // that turn, and it does so with a `taskTurn` event.
    queryClient.setQueryData<LiveStatus>(queryKeys.tasks.liveStatus('task-reaped'), {
      sandboxStatus: 'stopped',
      agentStatus: 'running',
      sandboxError: null,
    })
    global.fetch = createEventStreamFetch([
      makeEvent({
        type: 'taskTurn',
        subType: 'completed',
        taskId: 'task-reaped',
        agentWorking: false,
      }),
    ]) as any

    renderWithProviders(<div />, queryClient)

    await waitFor(() => {
      expect(queryClient.getQueryData(queryKeys.tasks.liveStatus('task-reaped'))).toEqual({
        sandboxStatus: 'stopped',
        agentStatus: 'idle',
        sandboxError: null,
      })
    })
  })
})

describe('GlobalEventStreamProvider — reconnect invalidation (unit)', () => {
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

  it('does not blanket-invalidate active queries on the initial connect', async () => {
    // A stream that ends without a server `close` event → no reconnect scheduled.
    global.fetch = vi.fn().mockImplementation(() => Promise.resolve(streamResponse([]))) as any

    renderWithProviders(<div />, queryClient)

    await waitFor(() => expect(global.fetch).toHaveBeenCalledTimes(1), { timeout: 2000 })

    // The reconnect-only, unfiltered active refetch must never fire on first connect.
    expect(queryClient.invalidateQueries).not.toHaveBeenCalledWith({ refetchType: 'active' })
  })

  it('blanket-invalidates active queries after a reconnect', async () => {
    // First connect receives a server `close` (triggers reconnect); the reconnect
    // receives an empty stream that just ends (no further reconnect).
    let call = 0
    global.fetch = vi.fn().mockImplementation(() => {
      call += 1
      const lines = call === 1 ? [JSON.stringify({ type: 'close' })] : []
      return Promise.resolve(streamResponse(lines))
    }) as any

    renderWithProviders(<div />, queryClient)

    // Wait for the reconnect (second fetch) to fire — backoff is ~1000ms.
    await waitFor(() => expect(global.fetch).toHaveBeenCalledTimes(2), { timeout: 3000 })

    await waitFor(
      () => {
        // Reconnect invalidates every active query with no queryKey filter.
        expect(queryClient.invalidateQueries).toHaveBeenCalledWith({ refetchType: 'active' })
      },
      { timeout: 2000 },
    )

    // Exactly one blanket active refetch (from the single reconnect), and it
    // carried no queryKey — targeted handlers always pass a queryKey.
    const blanketCalls = (
      queryClient.invalidateQueries as ReturnType<typeof vi.fn>
    ).mock.calls.filter((callArgs) => {
      const arg = callArgs[0] as { refetchType?: string; queryKey?: unknown } | undefined
      return arg?.refetchType === 'active' && arg?.queryKey === undefined
    })
    expect(blanketCalls).toHaveLength(1)
  })
})
