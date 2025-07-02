import type { Event } from '@moatless/shared/types/event'
import { EventType } from '@moatless/shared/types/event'
import { useQueryClient } from '@tanstack/react-query'
import type React from 'react'
import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { queryKeys } from '@/lib/api/query-keys'
import { getInvalidationKeys, shouldInvalidateTaskList } from '../utils/event-handlers'

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

  const eventSourceRef = useRef<EventSource | null>(null)
  const reconnectTimeoutRef = useRef<NodeJS.Timeout | undefined>(undefined)
  const reconnectAttemptsRef = useRef(0)
  const queryClient = useQueryClient()

  // Event handlers
  const taskHandlersRef = useRef<Map<string, Set<(event: Event) => void>>>(new Map())
  const allHandlersRef = useRef<Set<(event: Event) => void>>(new Set())
  const typeHandlersRef = useRef<Map<EventType, Set<(event: Event) => void>>>(new Map())

  const handleEvent = useCallback(
    (event: Event) => {
      // Update local state
      setLastEvent(event)

      // Store event by task
      const taskEvents = eventsByTask.get(event.taskId) || []
      taskEvents.push(event)
      eventsByTask.set(event.taskId, taskEvents)

      // Use smart invalidation mapping
      const invalidationKeys = getInvalidationKeys(event)
      
      if (invalidationKeys.length > 0) {
        console.log(`[GlobalEventStream] Event ${event.type} triggering invalidations:`, invalidationKeys.map(fn => fn.name))
        
        // Use queueMicrotask to ensure state updates complete before invalidation
        queueMicrotask(() => {
          // Invalidate each affected query
          invalidationKeys.forEach(keyFunction => {
            queryClient.invalidateQueries({
              queryKey: keyFunction(event.taskId),
              refetchType: 'active',
            })
          })
          
          // Always invalidate task lists for status-affecting events
          if (shouldInvalidateTaskList(event)) {
            queryClient.invalidateQueries({
              queryKey: queryKeys.tasksList(),
              refetchType: 'active',
            })
          }
        })
      }

      // Handle subType-specific invalidations (legacy support)
      if (event.subType) {
        switch (event.subType) {
          case 'iteration:created':
          case 'iteration:updated':
          case 'task:created':
          case 'task:updated':
          case 'task:statusChanged':
            queryClient.invalidateQueries({ queryKey: queryKeys.task(event.taskId) })
            queryClient.invalidateQueries({ queryKey: queryKeys.tasksList() })
            break

          case 'tool:start':
          case 'tool:result':
          case 'tool:error':
            queryClient.invalidateQueries({ queryKey: queryKeys.taskMessages(event.taskId) })
            queryClient.invalidateQueries({ queryKey: queryKeys.taskEvents(event.taskId) })
            break
        }
      }

      // Notify subscribers
      const taskHandlers = taskHandlersRef.current.get(event.taskId)
      if (taskHandlers) {
        taskHandlers.forEach((handler) => handler(event))
      }

      allHandlersRef.current.forEach((handler) => handler(event))

      const typeHandlers = typeHandlersRef.current.get(event.type)
      if (typeHandlers) {
        typeHandlers.forEach((handler) => handler(event))
      }
    },
    [eventsByTask, queryClient],
  )

  const connect = useCallback(() => {
    if (!enabled || eventSourceRef.current) return

    try {
      const params = new URLSearchParams()
      if (types?.length) params.append('types', types.join(','))

      const url = `/api/v1/events/stream?${params.toString()}`
      const eventSource = new EventSource(url)

      eventSource.onopen = () => {
        setIsConnected(true)
        setConnectionError(null)
        reconnectAttemptsRef.current = 0
        console.log('[GlobalEventStream] Connected')
      }

      eventSource.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data)

          // Ignore ping events
          if (data.type === 'ping') return

          handleEvent(data as Event)
        } catch (error) {
          console.error('[GlobalEventStream] Failed to parse event:', error)
        }
      }

      eventSource.onerror = (error) => {
        console.error('[GlobalEventStream] Connection error:', error)
        setIsConnected(false)

        if (eventSource.readyState === EventSource.CLOSED) {
          setConnectionError(new Error('Connection closed'))
          eventSource.close()
          eventSourceRef.current = null

          // Exponential backoff reconnect
          const attempts = reconnectAttemptsRef.current
          const delay = Math.min(1000 * 2 ** attempts, 30000)

          console.log(`[GlobalEventStream] Reconnecting in ${delay}ms (attempt ${attempts + 1})`)
          reconnectTimeoutRef.current = setTimeout(() => {
            reconnectAttemptsRef.current++
            connect()
          }, delay)
        }
      }

      eventSourceRef.current = eventSource
    } catch (error) {
      console.error('[GlobalEventStream] Failed to create connection:', error)
      setConnectionError(error instanceof Error ? error : new Error('Failed to connect'))
      toast.error('Failed to connect to event stream')
    }
  }, [enabled, types, handleEvent])

  const disconnect = useCallback(() => {
    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current)
      reconnectTimeoutRef.current = undefined
    }

    if (eventSourceRef.current) {
      eventSourceRef.current.close()
      eventSourceRef.current = null
      setIsConnected(false)
      console.log('[GlobalEventStream] Disconnected')
    }
  }, [])

  // Subscribe functions
  const subscribeToTask = useCallback((taskId: string, handler: (event: Event) => void) => {
    if (!taskHandlersRef.current.has(taskId)) {
      taskHandlersRef.current.set(taskId, new Set())
    }
    taskHandlersRef.current.get(taskId)!.add(handler)

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
    typeHandlersRef.current.get(type)!.add(handler)

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

  // Connection lifecycle
  useEffect(() => {
    connect()
    return () => {
      disconnect()
    }
  }, [connect, disconnect])

  // Visibility change handling
  useEffect(() => {
    const handleVisibilityChange = () => {
      if (document.hidden) {
        disconnect()
      } else {
        connect()
      }
    }

    document.addEventListener('visibilitychange', handleVisibilityChange)
    return () => {
      document.removeEventListener('visibilitychange', handleVisibilityChange)
    }
  }, [connect, disconnect])

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
    if (!taskId) return

    return subscribeToTask(taskId, handler)
  }, [taskId, handler, subscribeToTask])
}

export function useEventTypeSubscription(type: EventType, handler: (event: Event) => void) {
  const { subscribeToType } = useGlobalEventStream()

  useEffect(() => {
    return subscribeToType(type, handler)
  }, [type, handler, subscribeToType])
}
