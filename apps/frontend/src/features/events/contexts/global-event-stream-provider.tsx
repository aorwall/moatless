import type { ResourceMetrics, SandboxStatusFullResponse, ServerEntry } from '@moatless/api-client'
import { useQueryClient } from '@tanstack/react-query'
import type React from 'react'
import { createContext, useContext, useEffect, useEffectEvent, useRef, useState } from 'react'
import { toast } from 'sonner'
import {
  getInvalidationKeys,
  invalidateByTool,
  invalidateMessagesForEvent,
  shouldInvalidateTaskList,
} from '@/features/events/utils/event-handlers'
import {
  applyTaskEventUpdate,
  applyTaskMetadataUpdate,
  taskFromEventPayload,
} from '@/features/tasks/utils/query-helpers'
import { queryKeys } from '@/lib/api/query-keys'
import type { Event, EventType } from '@/lib/types/events'

/**
 * Validates that a parsed JSON value has the required SSE event envelope fields.
 * Only checks the base envelope — deep payload shape validation is intentionally
 * omitted to avoid schema drift.
 */
function isStreamEvent(value: unknown): value is Event {
  if (typeof value !== 'object' || value === null) {
    return false
  }
  const v = value as Record<string, unknown>
  return (
    typeof v['eventId'] === 'string' &&
    typeof v['taskId'] === 'string' &&
    typeof v['type'] === 'string' &&
    typeof v['subType'] === 'string' &&
    typeof v['isError'] === 'boolean' &&
    'payload' in v
  )
}

function updateSandboxStatus(
  event: Event,
  old: SandboxStatusFullResponse | undefined,
): SandboxStatusFullResponse | undefined {
  const current: SandboxStatusFullResponse = old ?? {
    sandboxState: 'missing',
    environmentStatus: 'not_created',
    agentState: 'none',
    servers: [],
    sandboxReady: false,
    canExecute: false,
  }

  switch (event.type) {
    case 'agent':
      if (event.subType === 'started') {
        return { ...current, agentState: 'running' }
      }
      if (
        event.subType === 'completed' ||
        event.subType === 'stopped' ||
        event.subType === 'error'
      ) {
        return { ...current, agentState: 'idle' }
      }
      return current
    case 'sandbox': {
      const validStatuses = [
        'not_created',
        'initializing',
        'ready',
        'stopped',
        'removing',
        'removed',
        'error',
      ]
      if (validStatuses.includes(event.subType || '')) {
        return {
          ...current,
          environmentStatus: event.subType as
            | 'not_created'
            | 'initializing'
            | 'ready'
            | 'stopped'
            | 'removing'
            | 'removed'
            | 'error',
          sandboxReady: event.subType === 'ready',
          canExecute: event.subType === 'ready' && current.agentState === 'idle',
        }
      }
      const validStates = ['exists', 'missing']
      if (validStates.includes(event.subType || '')) {
        return { ...current, sandboxState: event.subType as 'exists' | 'missing' }
      }
      return current
    }
    case 'task':
    case 'taskTurn':
    case 'server':
    case 'resources':
    case 'navigation':
    case 'preview':
      return current
  }
}

function isCloseEvent(line: string): boolean {
  try {
    const data = JSON.parse(line)
    return data.type === 'close'
  } catch {
    return false
  }
}

interface GlobalEventStreamContextType {
  isConnected: boolean
  connectionError: Error | null
  lastEvent: Event | null
  eventsByTask: Map<string, Event[]>
  subscribeToTask: (taskId: string, handler: (event: Event) => void) => () => void
  subscribeToAll: (handler: (event: Event) => void) => () => void
  subscribeToType: (type: EventType, handler: (event: Event) => void) => () => void
}

const GlobalEventStreamContext = createContext<GlobalEventStreamContextType | null>(null)

interface GlobalEventStreamProviderProps {
  children: React.ReactNode
  enabled?: boolean
  types?: EventType[]
}

export function GlobalEventStreamProvider({
  children,
  enabled = true,
  types,
}: GlobalEventStreamProviderProps) {
  const streamTypes = types?.join(',')
  const [isConnected, setIsConnected] = useState(false)
  const [connectionError, setConnectionError] = useState<Error | null>(null)
  const [lastEvent, setLastEvent] = useState<Event | null>(null)
  const [eventsByTask] = useState(() => new Map<string, Event[]>())

  const abortControllerRef = useRef<AbortController | null>(null)
  const reconnectTimeoutRef = useRef<NodeJS.Timeout | undefined>(undefined)
  const reconnectAttemptsRef = useRef(0)
  const isConnectingRef = useRef(false)
  const isManualDisconnectRef = useRef(false)
  const connectRef = useRef<(() => void) | null>(null)
  const queryClient = useQueryClient()

  // Event handlers
  const taskHandlersRef = useRef<Map<string, Set<(event: Event) => void>>>(new Map())
  const allHandlersRef = useRef<Set<(event: Event) => void>>(new Set())
  const typeHandlersRef = useRef<Map<EventType, Set<(event: Event) => void>>>(new Map())

  // Event deduplication - prevent processing same event multiple times
  const processedEventIds = useRef<Set<string>>(new Set())

  // Helper: Handle task.updated / task.metadata.updated events. These carry the
  // changed name/description in the payload, so patch the detail + list caches
  // directly instead of refetching. Fall back to invalidation only if the
  // payload has no recognized fields (keeps us correct for unknown future
  // event shapes).
  const handleTaskMetadataUpdate = (event: Event) => {
    if (applyTaskMetadataUpdate(queryClient, event)) {
      return
    }
    queryClient.invalidateQueries({
      queryKey: queryKeys.tasks.detail(event.taskId),
    })
    queryClient.invalidateQueries({
      queryKey: queryKeys.tasks.lists(),
      refetchType: 'active',
    })
  }

  // Helper: Apply cache updates and invalidations for events with taskId
  const applyCacheUpdates = (event: Event) => {
    if (!event.taskId) {
      return
    }

    if (
      event.type === 'task' &&
      (event.subType === 'updated' || event.subType === 'metadata.updated')
    ) {
      handleTaskMetadataUpdate(event)
      return // Handled directly (or via fallback); skip generic paths
    }

    // Apply optimistic updates for sandbox status events
    if (event.type === 'agent' || event.type === 'sandbox') {
      const queryKey = queryKeys.tasks.sandboxStatus(event.taskId)
      queryClient.setQueryData(queryKey, (old: SandboxStatusFullResponse | undefined) =>
        updateSandboxStatus(event, old),
      )
    }

    // Apply optimistic updates for server events — update the matching server
    // entry inside the sandbox status `servers` array by serverName
    if (event.type === 'server') {
      const wsKey = queryKeys.tasks.sandboxStatus(event.taskId)
      queryClient.setQueryData(wsKey, (old: SandboxStatusFullResponse | undefined) => {
        if (!old) {
          return old
        }
        const serverName = (event.payload as { serverName?: string }).serverName
        if (!serverName) {
          return old
        }
        const updatedServers = old.servers.map((s: ServerEntry) =>
          s.name === serverName
            ? { ...s, status: event.payload.status, error: event.payload.message }
            : s,
        )
        return { ...old, servers: updatedServers }
      })
    }

    // Apply optimistic updates for resource metrics events
    if (event.type === 'resources') {
      const wsKey = queryKeys.tasks.sandboxStatus(event.taskId)
      queryClient.setQueryData(wsKey, (old: SandboxStatusFullResponse | undefined) => {
        if (!old) {
          return old
        }
        return { ...old, resources: event.payload as ResourceMetrics }
      })
    }

    // Apply optimistic updates for task-related events
    if (event.type === 'task' || event.type === 'taskTurn' || event.type === 'agent') {
      applyTaskEventUpdate(queryClient, event)
    }

    // Invalidate queries based on tools used in the event
    invalidateByTool(event, queryClient)

    // Targeted refresh of the newest messages page (no full infinite query
    // invalidation) for events that change what the user is actively reading.
    invalidateMessagesForEvent(event, queryClient)

    const invalidationKeys = getInvalidationKeys(event)

    if (invalidationKeys.length > 0) {
      // Use queueMicrotask to ensure state updates complete before invalidation
      queueMicrotask(() => {
        // Only invalidate non-task queries - task data is handled optimistically above
        const nonTaskKeys = invalidationKeys.filter((keyFunction) => {
          const sampleKey = keyFunction(event.taskId)
          // Skip task detail invalidations - handled optimistically
          // But allow messages, events, and other sub-resource queries to be invalidated
          const isTaskDetail = sampleKey.length === 3 && sampleKey[1] === 'detail'
          return !isTaskDetail
        })

        for (const keyFunction of nonTaskKeys) {
          queryClient.invalidateQueries({
            queryKey: keyFunction(event.taskId),
          })
        }

        // Refetch task lists only for `task` events, which can change list
        // membership (a new task appears via `task.created`, a task enters or
        // leaves the status-filtered list on `task.open`/`task.closed`) —
        // things optimistic in-place patches can't do, and the only way other
        // clients learn about a new task.
        //
        // High-frequency agent/taskTurn/sandbox/server events do NOT change
        // list membership: running indicators are derived from the raw event
        // stream (useTaskActivity) and per-task sandboxStatus queries, and
        // status/lastMessageAt are patched in place by applyTaskEventUpdate.
        // Refetching the whole sidebar on each of those during an active run
        // was redundant, so it's dropped here.
        if (
          event.type === 'task' &&
          shouldInvalidateTaskList(event) &&
          !taskFromEventPayload(event) &&
          ['created', 'updated', 'open', 'closed'].includes(event.subType)
        ) {
          queryClient.invalidateQueries({
            queryKey: queryKeys.tasks.lists(),
            refetchType: 'active',
          })
        }
      })
    }
  }

  // Helper: Notify all subscribers about the event
  const notifySubscribers = (event: Event) => {
    const taskHandlers = taskHandlersRef.current.get(event.taskId)
    const typeHandlers = typeHandlersRef.current.get(event.type)

    if (taskHandlers) {
      for (const handler of taskHandlers) {
        handler(event)
      }
    }

    for (const handler of allHandlersRef.current) {
      handler(event)
    }

    if (typeHandlers) {
      for (const handler of typeHandlers) {
        handler(event)
      }
    }
  }

  const handleEvent = (event: Event) => {
    // Deduplicate events - skip if already processed
    if (event.eventId && processedEventIds.current.has(event.eventId)) {
      return
    }

    // Track processed event
    if (event.eventId) {
      processedEventIds.current.add(event.eventId)

      // Limit set size to prevent memory leak (keep last 500 events)
      if (processedEventIds.current.size > 1000) {
        const arr = Array.from(processedEventIds.current)
        processedEventIds.current = new Set(arr.slice(-500))
      }
    }

    setLastEvent(event)

    // Store event by task
    const taskEvents = eventsByTask.get(event.taskId) || []
    taskEvents.push(event)
    eventsByTask.set(event.taskId, taskEvents)

    // Apply cache updates and invalidations
    applyCacheUpdates(event)

    // Notify subscribers
    notifySubscribers(event)
  }

  // Helper: Process a single NDJSON line
  const processNDJSONLine = (line: string) => {
    if (!line.trim()) {
      return
    }

    try {
      const data: unknown = JSON.parse(line)

      // Ignore ping events (checked before full validation — pings are intentionally minimal)
      if (
        typeof data === 'object' &&
        data !== null &&
        (data as Record<string, unknown>)['type'] === 'ping'
      ) {
        return
      }

      if (!isStreamEvent(data)) {
        // biome-ignore lint/suspicious/noConsole: surfacing discarded malformed events aids debugging
        console.warn(
          '[EventStream] Discarding malformed event (missing required envelope fields):',
          line.slice(0, 200),
        )
        return
      }

      handleEvent(data)
    } catch (_error) {
      // Silently ignore JSON parse errors for malformed events
    }
  }

  // Helper: Read stream and process NDJSON lines
  const processStream = async (
    reader: ReadableStreamDefaultReader<Uint8Array>,
  ): Promise<{ shouldReconnect: boolean }> => {
    const decoder = new TextDecoder()
    let buffer = ''
    let receivedCloseEvent = false

    while (true) {
      const { done, value } = await reader.read()

      if (done) {
        break
      }

      // Decode the chunk and add to buffer
      buffer += decoder.decode(value, { stream: true })

      // Process complete lines (NDJSON format)
      const lines = buffer.split('\n')
      buffer = lines.pop() || '' // Keep incomplete line in buffer

      for (const line of lines) {
        if (!line.trim()) {
          continue
        }

        // Check for close event from server
        if (isCloseEvent(line)) {
          receivedCloseEvent = true
          continue
        }

        processNDJSONLine(line)
      }
    }

    // Reconnect if we received a close event from the server
    return { shouldReconnect: receivedCloseEvent }
  }

  // Helper: Schedule reconnection with exponential backoff
  const scheduleReconnect = (onReconnect: () => void) => {
    const attempts = reconnectAttemptsRef.current
    const delay = Math.min(1000 * 2 ** attempts, 30_000)
    reconnectTimeoutRef.current = setTimeout(() => {
      reconnectAttemptsRef.current++
      onReconnect()
    }, delay)
  }

  // Helper: Handle successful fetch response
  const handleFetchResponse = async (response: Response) => {
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${response.statusText}`)
    }

    if (!response.body) {
      throw new Error('No response body')
    }

    // Check if this is a reconnection (not initial connection)
    const wasReconnecting = reconnectAttemptsRef.current > 0

    setIsConnected(true)
    setConnectionError(null)
    reconnectAttemptsRef.current = 0
    isConnectingRef.current = false

    // Notify user on successful reconnection
    if (wasReconnecting) {
      // Events emitted while disconnected are gone (the backend has no
      // Last-Event-ID replay), so anything observed on screen may be stale.
      // refetchType: 'active' limits this to mounted queries — dormant cache
      // entries stay marked stale and refetch naturally on next observation.
      // No queryKey filter: the query-key namespaces don't share a common
      // root, so invalidating everything is the only way to cover them all.
      queryClient.invalidateQueries({ refetchType: 'active' })
      toast.success('Reconnected to server')
    }

    const reader = response.body.getReader()

    try {
      const { shouldReconnect } = await processStream(reader)

      // After stream ends, check if we should reconnect
      if (shouldReconnect && !isManualDisconnectRef.current && connectRef.current) {
        setIsConnected(false)
        isConnectingRef.current = false
        toast.info('Server disconnected. Reconnecting...')
        scheduleReconnect(connectRef.current)
      }
    } catch (error) {
      if (error instanceof Error && error.name === 'AbortError') {
        return
      }
      throw error
    }
  }

  // Helper: Handle fetch error and reconnection
  const handleFetchError = (
    error: unknown,
    abortController: AbortController,
    onReconnect: () => void,
  ) => {
    setIsConnected(false)
    isConnectingRef.current = false

    // Only reconnect if the connection wasn't manually aborted
    if (!abortController.signal.aborted && error instanceof Error && error.name !== 'AbortError') {
      setConnectionError(error)
      scheduleReconnect(onReconnect)
    }
    // Connection was manually aborted - no action needed
  }

  // Helper: Build stream URL with query params
  // Always uses a relative URL — Vite proxy (dev) and Traefik (prod) route to the Rust backend
  const buildStreamUrl = (): string => {
    const params = new URLSearchParams()
    if (streamTypes) {
      params.append('types', streamTypes)
    }
    const query = params.toString()
    return `/api/v1/events/stream${query ? `?${query}` : ''}`
  }

  const connect = () => {
    if (!enabled) {
      return
    }

    // Prevent concurrent connections
    if (isConnectingRef.current) {
      return
    }

    // Clean up any existing connection first
    if (abortControllerRef.current) {
      try {
        abortControllerRef.current.abort()
      } catch (_error) {
        // Abort may throw if already aborted - safe to ignore
      }
      abortControllerRef.current = null
      setIsConnected(false)
    }

    // Clear any pending reconnect
    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current)
      reconnectTimeoutRef.current = undefined
    }
    isConnectingRef.current = true
    isManualDisconnectRef.current = false

    try {
      const url = buildStreamUrl()
      const abortController = new AbortController()
      abortControllerRef.current = abortController

      fetch(url, {
        method: 'GET',
        headers: {
          Accept: 'text/plain',
          'Cache-Control': 'no-cache',
        },
        credentials: 'include',
        signal: abortController.signal,
      })
        .then((response) => handleFetchResponse(response))
        .catch((error) => handleFetchError(error, abortController, () => connectRef.current?.()))
    } catch (error) {
      setConnectionError(error instanceof Error ? error : new Error('Failed to connect'))
      isConnectingRef.current = false
      toast.error('Failed to connect to event stream')
    }
  }

  const disconnect = () => {
    // Mark as manual disconnect to prevent automatic reconnection
    isManualDisconnectRef.current = true

    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current)
      reconnectTimeoutRef.current = undefined
    }

    if (abortControllerRef.current) {
      try {
        abortControllerRef.current.abort()
      } catch (_error) {
        // Abort may throw if already aborted - safe to ignore
      }
      abortControllerRef.current = null
      setIsConnected(false)
    }

    isConnectingRef.current = false
  }

  // Subscribe functions
  const subscribeToTask = (taskId: string, handler: (event: Event) => void) => {
    if (!taskHandlersRef.current.has(taskId)) {
      taskHandlersRef.current.set(taskId, new Set())
    }
    const handlers = taskHandlersRef.current.get(taskId)
    if (handlers) {
      handlers.add(handler)
    }

    return () => {
      const handlers = taskHandlersRef.current.get(taskId)
      if (handlers) {
        handlers.delete(handler)
        if (handlers.size === 0) {
          taskHandlersRef.current.delete(taskId)
        }
      }
    }
  }

  const subscribeToAll = (handler: (event: Event) => void) => {
    allHandlersRef.current.add(handler)
    return () => {
      allHandlersRef.current.delete(handler)
    }
  }

  const subscribeToType = (type: EventType, handler: (event: Event) => void) => {
    if (!typeHandlersRef.current.has(type)) {
      typeHandlersRef.current.set(type, new Set())
    }
    const handlers = typeHandlersRef.current.get(type)
    if (handlers) {
      handlers.add(handler)
    }

    return () => {
      const handlers = typeHandlersRef.current.get(type)
      if (handlers) {
        handlers.delete(handler)
        if (handlers.size === 0) {
          typeHandlersRef.current.delete(type)
        }
      }
    }
  }

  const syncConnectRef = useEffectEvent((_streamTypes: string | undefined) => {
    connectRef.current = connect
  })

  const connectEvent = useEffectEvent(connect)
  const disconnectEvent = useEffectEvent(disconnect)

  // Start a fresh stream when its enabled state or event-type filter changes.
  useEffect(() => {
    if (!enabled) {
      disconnectEvent()
      return
    }

    let isMounted = true
    syncConnectRef(streamTypes)

    // Small delay to handle React StrictMode double-mount
    const timer = setTimeout(() => {
      if (isMounted) {
        connectEvent()
      }
    }, 10)

    // Cleanup on unmount
    return () => {
      isMounted = false
      clearTimeout(timer)
      disconnectEvent()
    }
  }, [enabled, streamTypes])

  // Note: We don't handle visibility changes to ensure a stable connection
  // The stream will remain connected even when the tab is in the background

  const value = {
    isConnected,
    connectionError,
    lastEvent,
    eventsByTask,
    subscribeToTask,
    subscribeToAll,
    subscribeToType,
  }

  return <GlobalEventStreamContext value={value}>{children}</GlobalEventStreamContext>
}

export function useGlobalEventStream() {
  const context = useContext(GlobalEventStreamContext)
  if (!context) {
    throw new Error('useGlobalEventStream must be used within GlobalEventStreamProvider')
  }
  return context
}

// Convenience hooks
export function useTaskEventSubscription(
  taskId: string | undefined,
  handler: (event: Event) => void,
) {
  const { subscribeToTask } = useGlobalEventStream()
  const handleEvent = useEffectEvent(handler)

  useEffect(() => {
    if (!taskId) {
      return
    }

    return subscribeToTask(taskId, (event) => handleEvent(event))
  }, [taskId, subscribeToTask])
}
