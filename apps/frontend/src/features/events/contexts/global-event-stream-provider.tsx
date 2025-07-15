import type { Event, EventType } from '@moatless/types'
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

  const abortControllerRef = useRef<AbortController | null>(null)
  const reconnectTimeoutRef = useRef<NodeJS.Timeout | undefined>(undefined)
  const reconnectAttemptsRef = useRef(0)
  const isConnectingRef = useRef(false)
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

      // Use smart invalidation mapping for all events with taskId
      if (event.taskId) {
        const invalidationKeys = getInvalidationKeys(event)

        if (invalidationKeys.length > 0) {
          // Use queueMicrotask to ensure state updates complete before invalidation
          queueMicrotask(() => {
            // Invalidate each affected query
            invalidationKeys.forEach((keyFunction) => {
              queryClient.invalidateQueries({
                queryKey: keyFunction(event.taskId),
              })
            })

            // Invalidate task lists for status-affecting events to update sidebar
            if (shouldInvalidateTaskList(event)) {
              queryClient.invalidateQueries({
                queryKey: queryKeys.tasks.lists(),
              })
            }
          })
        }
      }

      // Handle subType-specific invalidations (legacy support)
      if (event.taskId && event.subType) {
        switch (event.subType) {
          case 'iteration:created':
          case 'iteration:updated':
          case 'task:created':
          case 'task:updated':
            queryClient.invalidateQueries({ queryKey: queryKeys.tasks.detail(event.taskId) })
            queryClient.invalidateQueries({ queryKey: queryKeys.tasks.lists() })
            break

          case 'tool:start':
          case 'tool:result':
          case 'tool:error':
            queryClient.invalidateQueries({ queryKey: queryKeys.tasks.messages(event.taskId) })
            queryClient.invalidateQueries({ queryKey: queryKeys.tasks.events(event.taskId) })
            break
        }
      }

      // Notify subscribers
      const taskHandlers = taskHandlersRef.current.get(event.taskId)
      const typeHandlers = typeHandlersRef.current.get(event.type)
      const globalHandlers = allHandlersRef.current.size

      // Log event details and subscribers that will receive it
      const subscribers = [
        taskHandlers ? `task:${taskHandlers.size}` : null,
        typeHandlers ? `type:${typeHandlers.size}` : null,
        globalHandlers > 0 ? `global:${globalHandlers}` : null,
      ]
        .filter(Boolean)
        .join(', ')

      console.log(
        `[GlobalEventStream] Event: type=${event.type}, subType=${event.subType || 'none'}, taskId=${event.taskId}, subscribers=[${subscribers || 'none'}]`,
      )

      if (taskHandlers) {
        taskHandlers.forEach((handler) => handler(event))
      }

      allHandlersRef.current.forEach((handler) => handler(event))

      if (typeHandlers) {
        typeHandlers.forEach((handler) => handler(event))
      }
    },
    [eventsByTask, queryClient],
  )

  const connect = useCallback(() => {
    if (!enabled) return

    // Prevent concurrent connections
    if (isConnectingRef.current) {
      console.log('[GlobalEventStream] Connection already in progress, skipping')
      return
    }

    // Clean up any existing connection first
    if (abortControllerRef.current) {
      console.log('[GlobalEventStream] Cleaning up existing connection before reconnecting')
      try {
        abortControllerRef.current.abort()
      } catch (error) {
        console.log('[GlobalEventStream] Abort error during cleanup (expected):', error)
      }
      abortControllerRef.current = null
      setIsConnected(false)
    }

    // Clear any pending reconnect
    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current)
      reconnectTimeoutRef.current = undefined
    }

    console.log('[GlobalEventStream] Starting new connection...')
    isConnectingRef.current = true

    try {
      const params = new URLSearchParams()
      if (types?.length) params.append('types', types.join(','))

      const baseUrl = import.meta.env.VITE_API_URL || 'http://localhost:8000'
      const url = `${baseUrl}/api/v1/events/stream?${params.toString()}`

      // Use fetch with ReadableStream to handle NDJSON format
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
        .then(async (response) => {
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
          console.log('[GlobalEventStream] Connected')

          const reader = response.body.getReader()
          const decoder = new TextDecoder()
          let buffer = ''

          try {
            while (true) {
              const { done, value } = await reader.read()

              if (done) {
                console.log('[GlobalEventStream] Stream ended')
                break
              }

              // Decode the chunk and add to buffer
              buffer += decoder.decode(value, { stream: true })

              // Process complete lines (NDJSON format)
              const lines = buffer.split('\n')
              buffer = lines.pop() || '' // Keep incomplete line in buffer

              for (const line of lines) {
                if (line.trim()) {
                  try {
                    const data = JSON.parse(line)

                    // Ignore ping events
                    if (data.type === 'ping') {
                      continue
                    }

                    handleEvent(data as Event)
                  } catch (error) {
                    console.error(
                      '[GlobalEventStream] Failed to parse line:',
                      error,
                      'Raw line:',
                      line,
                    )
                  }
                }
              }
            }
          } catch (error) {
            if (error instanceof Error && error.name === 'AbortError') {
              console.log('[GlobalEventStream] Connection aborted')
              return
            }
            throw error
          }
        })
        .catch((error) => {
          console.error('[GlobalEventStream] Connection error:', error)
          setIsConnected(false)
          isConnectingRef.current = false

          // Only reconnect if the connection wasn't manually aborted
          if (!abortController.signal.aborted && error.name !== 'AbortError') {
            setConnectionError(error instanceof Error ? error : new Error('Connection failed'))

            // Exponential backoff reconnect
            const attempts = reconnectAttemptsRef.current
            const delay = Math.min(1000 * 2 ** attempts, 30000)

            console.log(`[GlobalEventStream] Reconnecting in ${delay}ms (attempt ${attempts + 1})`)
            reconnectTimeoutRef.current = setTimeout(() => {
              reconnectAttemptsRef.current++
              connect()
            }, delay)
          } else {
            console.log('[GlobalEventStream] Connection aborted, not reconnecting')
          }
        })
    } catch (error) {
      console.error('[GlobalEventStream] Failed to create connection:', error)
      setConnectionError(error instanceof Error ? error : new Error('Failed to connect'))
      isConnectingRef.current = false
      toast.error('Failed to connect to event stream')
    }
  }, [enabled, types, handleEvent])

  const disconnect = useCallback(() => {
    console.log('[GlobalEventStream] Disconnecting...')

    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current)
      reconnectTimeoutRef.current = undefined
    }

    if (abortControllerRef.current) {
      try {
        abortControllerRef.current.abort()
      } catch (error) {
        console.log('[GlobalEventStream] Abort error (expected):', error)
      }
      abortControllerRef.current = null
      setIsConnected(false)
      console.log('[GlobalEventStream] Disconnected')
    }

    isConnectingRef.current = false
  }, [])

  // Subscribe functions
  const subscribeToTask = useCallback((taskId: string, handler: (event: Event) => void) => {
    if (!taskHandlersRef.current.has(taskId)) {
      taskHandlersRef.current.set(taskId, new Set())
    }
    taskHandlersRef.current.get(taskId)!.add(handler)
    console.log(
      `[GlobalEventStream] Subscribed to task: ${taskId}, total handlers: ${taskHandlersRef.current.get(taskId)!.size}`,
    )

    return () => {
      const handlers = taskHandlersRef.current.get(taskId)
      if (handlers) {
        handlers.delete(handler)
        if (handlers.size === 0) {
          taskHandlersRef.current.delete(taskId)
        }
        console.log(
          `[GlobalEventStream] Unsubscribed from task: ${taskId}, remaining handlers: ${handlers.size}`,
        )
      }
    }
  }, [])

  const subscribeToAll = useCallback((handler: (event: Event) => void) => {
    allHandlersRef.current.add(handler)
    console.log(
      `[GlobalEventStream] Subscribed to all events, total handlers: ${allHandlersRef.current.size}`,
    )
    return () => {
      allHandlersRef.current.delete(handler)
      console.log(
        `[GlobalEventStream] Unsubscribed from all events, remaining handlers: ${allHandlersRef.current.size}`,
      )
    }
  }, [])

  const subscribeToType = useCallback((type: EventType, handler: (event: Event) => void) => {
    if (!typeHandlersRef.current.has(type)) {
      typeHandlersRef.current.set(type, new Set())
    }
    typeHandlersRef.current.get(type)!.add(handler)
    console.log(
      `[GlobalEventStream] Subscribed to type: ${type}, total handlers: ${typeHandlersRef.current.get(type)!.size}`,
    )

    return () => {
      const handlers = typeHandlersRef.current.get(type)
      if (handlers) {
        handlers.delete(handler)
        if (handlers.size === 0) {
          typeHandlersRef.current.delete(type)
        }
        console.log(
          `[GlobalEventStream] Unsubscribed from type: ${type}, remaining handlers: ${handlers.size}`,
        )
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
