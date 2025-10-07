import type { Event, EventType, WorkspaceStatusResponse } from '@moatless/types'
import { useQueryClient } from '@tanstack/react-query'
import type React from 'react'
import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { applyTaskEventUpdate } from '@/features/tasks/utils/query-helpers'
import { queryKeys } from '@/lib/api/query-keys'
import { getInvalidationKeys } from '../utils/event-handlers'

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
  const [lastEvent, setLastEvent] = useState<Event | null>(null)
  const [eventsByTask] = useState(() => new Map<string, Event[]>())

  const abortControllerRef = useRef<AbortController | null>(null)
  const reconnectTimeoutRef = useRef<NodeJS.Timeout | undefined>(undefined)
  const reconnectAttemptsRef = useRef(0)
  const isConnectingRef = useRef(false)
  const queryClient = useQueryClient()

  // Event handlers
  const taskHandlersRef = useRef<Map<string, Set<(event: Event) => void>>>(new Map())
  const allHandlersRef = useRef<Set<(event: Event) => void>>(new Set())
  const typeHandlersRef = useRef<Map<EventType, Set<(event: Event) => void>>>(new Map())

  // Helper: Update workspace status based on event type
  const updateWorkspaceStatus = useCallback(
    (
      event: Event,
      old: WorkspaceStatusResponse | undefined,
    ): WorkspaceStatusResponse | undefined => {
      if (!old) {
        return old
      }

      switch (event.type) {
        case 'agent':
          if (event.subType === 'started') {
            return { ...old, agentState: 'running', isRunning: true }
          }
          if (event.subType === 'stopped' || event.subType === 'error') {
            return { ...old, agentState: 'idle', isRunning: false }
          }
          break
        case 'environment': {
          // Only update if subType is a valid EnvironmentStatus
          const validStatuses = ['not_created', 'initializing', 'ready', 'cleaning', 'error']
          if (validStatuses.includes(event.subType || '')) {
            return {
              ...old,
              environmentStatus: event.subType as
                | 'not_created'
                | 'initializing'
                | 'ready'
                | 'cleaning'
                | 'error',
              workspaceReady: event.subType === 'ready',
              canExecute: event.subType === 'ready' && old.agentState === 'idle',
            }
          }
          return old
        }
        case 'workspace': {
          // Only update if subType is a valid WorkspaceExistence
          const validStates = ['exists', 'missing', 'invalid']
          if (validStates.includes(event.subType || '')) {
            return { ...old, workspaceState: event.subType as 'exists' | 'missing' | 'invalid' }
          }
          return old
        }
      }
      return old
    },
    [],
  )

  // Helper: Apply cache updates and invalidations for events with taskId
  const applyCacheUpdates = useCallback(
    (event: Event) => {
      if (!event.taskId) {
        return
      }

      // Apply optimistic updates for workspace status events
      if (event.type === 'agent' || event.type === 'environment' || event.type === 'workspace') {
        const queryKey = queryKeys.tasks.workspaceStatus(event.taskId)
        queryClient.setQueryData(queryKey, (old: WorkspaceStatusResponse | undefined) =>
          updateWorkspaceStatus(event, old),
        )
      }

      // Apply optimistic updates for server events
      if (event.type === 'server') {
        const queryKey = queryKeys.tasks.serverStatus(event.taskId)
        queryClient.setQueryData(queryKey, (old) => {
          // Merge event payload with existing data to preserve fields like proxyUrl, logs, currentPath
          return {
            ...(old || {}),
            status: event.payload.status,
            url: event.payload.url,
            pid: event.payload.pid,
            uptimeSeconds: event.payload.uptimeSeconds,
            error: event.payload.error,
            // Keep existing proxyUrl, logs, currentPath from old data if available
          }
        })
      }

      // Apply optimistic updates for task-related events
      if (event.type === 'task' || event.type === 'taskIteration' || event.type === 'agent') {
        applyTaskEventUpdate(queryClient, event)
      }

      const invalidationKeys = getInvalidationKeys(event)

      if (invalidationKeys.length > 0) {
        // Use queueMicrotask to ensure state updates complete before invalidation
        queueMicrotask(() => {
          // Only invalidate non-task queries - task data is handled optimistically above
          const nonTaskKeys = invalidationKeys.filter((keyFunction) => {
            const sampleKey = keyFunction(event.taskId)
            // Skip task detail and task list invalidations - handled optimistically
            // But allow messages, events, and other sub-resource queries to be invalidated
            const isTaskDetail = sampleKey.length === 3 && sampleKey[1] === 'detail'
            const isTaskList = sampleKey[1] === 'list' || sampleKey[2] === 'list'
            return !(isTaskDetail || isTaskList)
          })

          for (const keyFunction of nonTaskKeys) {
            queryClient.invalidateQueries({
              queryKey: keyFunction(event.taskId),
            })
          }
        })
      }
    },
    [queryClient, updateWorkspaceStatus],
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
      // Update local state
      setLastEvent(event)

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
    async (reader: ReadableStreamDefaultReader<Uint8Array>) => {
      const decoder = new TextDecoder()
      let buffer = ''

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
          processNDJSONLine(line)
        }
      }
    },
    [processNDJSONLine],
  )

  // Helper: Handle successful fetch response
  const handleFetchResponse = useCallback(
    async (response: Response) => {
      if (!response.ok) {
        throw new Error(`HTTP ${response.status}: ${response.statusText}`)
      }

      if (!response.body) {
        throw new Error('No response body')
      }

      setIsConnected(true)
      setConnectionError(null)
      reconnectAttemptsRef.current = 0
      isConnectingRef.current = false

      const reader = response.body.getReader()

      try {
        await processStream(reader)
      } catch (error) {
        if (error instanceof Error && error.name === 'AbortError') {
          return
        }
        throw error
      }
    },
    [processStream],
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
        setConnectionError(error instanceof Error ? error : new Error('Connection failed'))
        scheduleReconnect(onReconnect)
      }
      // Connection was manually aborted - no action needed
    },
    [scheduleReconnect],
  )

  // Helper: Build stream URL with query params
  const buildStreamUrl = useCallback((): string => {
    const params = new URLSearchParams()
    if (types && types.length > 0) {
      params.append('types', types.join(','))
    }

    // Use same URL pattern as API client: relative URLs in production, absolute in development
    const baseUrl = import.meta.env.PROD
      ? '' // Empty string for relative URLs in production
      : import.meta.env.VITE_API_URL || 'http://localhost:8000'

    return `${baseUrl}/api/v1/events/stream?${params.toString()}`
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

  const value: GlobalEventStreamContextType = {
    isConnected,
    connectionError,
    lastEvent,
    eventsByTask,
    subscribeToTask,
    subscribeToAll,
    subscribeToType,
  }

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
