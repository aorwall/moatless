import type { ResourceMetrics, SandboxStatusFullResponse, ServerEntry } from '@moatless/api-client'
import { useQueryClient } from '@tanstack/react-query'
import type React from 'react'
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react'
import { toast } from 'sonner'
import {
  getInvalidationKeys,
  invalidateByTool,
  invalidateMessagesForEvent,
  shouldInvalidateTaskList,
} from '@/features/events'
import { applyTaskEventUpdate } from '@/features/tasks/utils/query-helpers'
import { queryKeys } from '@/lib/api/query-keys'
import type { Event, EventType } from '@/lib/types/events'

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
  const [isConnected, setIsConnected] = useState(false)
  const [connectionError, setConnectionError] = useState<Error | null>(null)
  const lastEventRef = useRef<Event | null>(null)
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

  // Helper: Update sandbox status based on event type
  const updateSandboxStatus = useCallback(
    (
      event: Event,
      old: SandboxStatusFullResponse | undefined,
    ): SandboxStatusFullResponse | undefined => {
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
          break
        case 'sandbox': {
          // Handle environment status updates
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
          // Handle workspace state updates (exists/missing)
          const validStates = ['exists', 'missing']
          if (validStates.includes(event.subType || '')) {
            return { ...current, sandboxState: event.subType as 'exists' | 'missing' }
          }
          return current
        }
      }
      return current
    },
    [],
  )

  // Helper: Apply cache updates and invalidations for events with taskId
  const applyCacheUpdates = useCallback(
    (event: Event) => {
      if (!event.taskId) {
        return
      }

      // Special case: task.updated events should force refetch (metadata changed)
      if (event.type === 'task' && event.subType === 'updated') {
        queryClient.invalidateQueries({
          queryKey: queryKeys.tasks.detail(event.taskId),
        })
        return // Skip optimistic updates for metadata changes
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

          // Fallback: invalidate task lists for status-affecting events
          // This ensures eventual consistency if optimistic updates fail
          if (shouldInvalidateTaskList(event)) {
            queryClient.invalidateQueries({
              queryKey: queryKeys.tasks.lists(),
              refetchType: 'active',
            })
          }
        })
      }
    },
    [queryClient, updateSandboxStatus],
  )

  // Helper: Notify all subscribers about the event
  const notifySubscribers = useCallback((event: Event) => {
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
  }, [])

  const handleEvent = useCallback(
    (event: Event) => {
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

      // Track last event (ref to avoid re-renders)
      lastEventRef.current = event

      // Store event by task
      const taskEvents = eventsByTask.get(event.taskId) || []
      taskEvents.push(event)
      eventsByTask.set(event.taskId, taskEvents)

      // Apply cache updates and invalidations
      applyCacheUpdates(event)

      // Notify subscribers
      notifySubscribers(event)
    },
    [eventsByTask, applyCacheUpdates, notifySubscribers],
  )

  // Helper: Check if a line is a close event from the server
  const isCloseEvent = useCallback((line: string): boolean => {
    try {
      const data = JSON.parse(line)
      return data.type === 'close'
    } catch {
      return false
    }
  }, [])

  // Helper: Process a single NDJSON line
  const processNDJSONLine = useCallback(
    (line: string) => {
      if (!line.trim()) {
        return
      }

      try {
        const data = JSON.parse(line)

        // Ignore ping events
        if (data.type === 'ping') {
          return
        }

        handleEvent(data as Event)
      } catch (_error) {
        // Silently ignore JSON parse errors for malformed events
      }
    },
    [handleEvent],
  )

  // Helper: Read stream and process NDJSON lines
  const processStream = useCallback(
    async (
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
    },
    [processNDJSONLine, isCloseEvent],
  )

  // Helper: Schedule reconnection with exponential backoff
  const scheduleReconnect = useCallback((onReconnect: () => void) => {
    const attempts = reconnectAttemptsRef.current
    const delay = Math.min(1000 * 2 ** attempts, 30000)
    reconnectTimeoutRef.current = setTimeout(() => {
      reconnectAttemptsRef.current++
      onReconnect()
    }, delay)
  }, [])

  // Helper: Handle successful fetch response
  const handleFetchResponse = useCallback(
    async (response: Response) => {
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
    },
    [processStream, scheduleReconnect],
  )

  // Helper: Handle fetch error and reconnection
  const handleFetchError = useCallback(
    (error: unknown, abortController: AbortController, onReconnect: () => void) => {
      setIsConnected(false)
      isConnectingRef.current = false

      // Only reconnect if the connection wasn't manually aborted
      if (
        !abortController.signal.aborted &&
        error instanceof Error &&
        error.name !== 'AbortError'
      ) {
        setConnectionError(error)
        scheduleReconnect(onReconnect)
      }
      // Connection was manually aborted - no action needed
    },
    [scheduleReconnect],
  )

  // Helper: Build stream URL with query params
  // Always uses a relative URL — Vite proxy (dev) and Traefik (prod) route to the Rust backend
  const buildStreamUrl = useCallback((): string => {
    const params = new URLSearchParams()
    if (types && types.length > 0) {
      params.append('types', types.join(','))
    }
    const query = params.toString()
    return `/api/v1/events/stream${query ? `?${query}` : ''}`
  }, [types])

  const connect = useCallback(() => {
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
        .catch((error) => handleFetchError(error, abortController, connect))
    } catch (error) {
      setConnectionError(error instanceof Error ? error : new Error('Failed to connect'))
      isConnectingRef.current = false
      toast.error('Failed to connect to event stream')
    }
  }, [enabled, buildStreamUrl, handleFetchResponse, handleFetchError])

  const disconnect = useCallback(() => {
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
  }, [])

  // Subscribe functions
  const subscribeToTask = useCallback((taskId: string, handler: (event: Event) => void) => {
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
  }, [])

  const subscribeToAll = useCallback((handler: (event: Event) => void) => {
    allHandlersRef.current.add(handler)
    return () => {
      allHandlersRef.current.delete(handler)
    }
  }, [])

  const subscribeToType = useCallback((type: EventType, handler: (event: Event) => void) => {
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
  }, [])

  // Keep connectRef up to date
  useEffect(() => {
    connectRef.current = connect
  }, [connect])

  // Connection lifecycle - connect once when mounted
  useEffect(() => {
    let isMounted = true

    // Small delay to handle React StrictMode double-mount
    const timer = setTimeout(() => {
      if (isMounted) {
        connect()
      }
    }, 10)

    // Cleanup on unmount
    return () => {
      isMounted = false
      clearTimeout(timer)
      disconnect()
    }
  }, [connect, disconnect]) // Include stable callback dependencies

  // Note: We don't handle visibility changes to ensure a stable connection
  // The stream will remain connected even when the tab is in the background

  const value = useMemo<GlobalEventStreamContextType>(
    () => ({
      isConnected,
      connectionError,
      lastEvent: lastEventRef.current,
      eventsByTask,
      subscribeToTask,
      subscribeToAll,
      subscribeToType,
    }),
    [isConnected, connectionError, eventsByTask, subscribeToTask, subscribeToAll, subscribeToType],
  )

  return (
    <GlobalEventStreamContext.Provider value={value}>{children}</GlobalEventStreamContext.Provider>
  )
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

  useEffect(() => {
    if (!taskId) {
      return
    }

    return subscribeToTask(taskId, handler)
  }, [taskId, handler, subscribeToTask])
}

export function useEventTypeSubscription(type: EventType, handler: (event: Event) => void) {
  const { subscribeToType } = useGlobalEventStream()

  useEffect(() => {
    return subscribeToType(type, handler)
  }, [type, handler, subscribeToType])
}
