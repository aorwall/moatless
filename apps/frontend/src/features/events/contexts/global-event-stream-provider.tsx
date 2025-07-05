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
      console.log('[GlobalEventStream] handleEvent called with:', {
        type: event.type,
        taskId: event.taskId,
        subType: event.subType,
        timestamp: event.timestamp,
        eventId: event.id,
        status: event.status,
        message: event.message
      })

      // Update local state
      setLastEvent(event)
      console.log('[GlobalEventStream] Updated lastEvent state')

      // Store event by task
      const taskEvents = eventsByTask.get(event.taskId) || []
      taskEvents.push(event)
      eventsByTask.set(event.taskId, taskEvents)
      console.log('[GlobalEventStream] Stored event for task:', event.taskId, 'Total events for task:', taskEvents.length)

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
        console.log('[GlobalEventStream] Notifying', taskHandlers.size, 'task-specific handlers for task:', event.taskId)
        taskHandlers.forEach((handler) => handler(event))
      }

      console.log('[GlobalEventStream] Notifying', allHandlersRef.current.size, 'global handlers')
      allHandlersRef.current.forEach((handler) => handler(event))

      const typeHandlers = typeHandlersRef.current.get(event.type)
      if (typeHandlers) {
        console.log('[GlobalEventStream] Notifying', typeHandlers.size, 'type-specific handlers for type:', event.type)
        typeHandlers.forEach((handler) => handler(event))
      } else {
        console.log('[GlobalEventStream] No type-specific handlers found for type:', event.type)
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
          'Accept': 'text/plain',
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
                    console.log('[GlobalEventStream] Raw line received:', line)
                    
                    const data = JSON.parse(line)
                    
                    console.log('[GlobalEventStream] Parsed event data:', {
                      type: data.type,
                      taskId: data.taskId,
                      subType: data.subType,
                      timestamp: data.timestamp,
                      fullEvent: data
                    })

                    // Ignore ping events
                    if (data.type === 'ping') {
                      console.log('[GlobalEventStream] Ignoring ping event')
                      continue
                    }

                    console.log('[GlobalEventStream] Processing event:', data.type, 'for task:', data.taskId)
                    handleEvent(data as Event)
                  } catch (error) {
                    console.error('[GlobalEventStream] Failed to parse line:', error, 'Raw line:', line)
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
    console.log('[GlobalEventStream] Subscribing to task events for task:', taskId)
    console.log('[GlobalEventStream] Subscribe stack trace:', new Error().stack?.split('\n').slice(2, 7).join('\n'))
    if (!taskHandlersRef.current.has(taskId)) {
      taskHandlersRef.current.set(taskId, new Set())
    }
    taskHandlersRef.current.get(taskId)!.add(handler)
    console.log('[GlobalEventStream] Total task handlers for', taskId, ':', taskHandlersRef.current.get(taskId)!.size)

    return () => {
      console.log('[GlobalEventStream] Unsubscribing from task events for task:', taskId)
      const handlers = taskHandlersRef.current.get(taskId)
      if (handlers) {
        handlers.delete(handler)
        if (handlers.size === 0) {
          taskHandlersRef.current.delete(taskId)
          console.log('[GlobalEventStream] Removed all handlers for task:', taskId)
        } else {
          console.log('[GlobalEventStream] Remaining task handlers for', taskId, ':', handlers.size)
        }
      }
    }
  }, [])

  const subscribeToAll = useCallback((handler: (event: Event) => void) => {
    console.log('[GlobalEventStream] Subscribing to all events', handler)
    console.log('[GlobalEventStream] Subscribe stack trace:', new Error().stack?.split('\n').slice(2, 7).join('\n'))
    allHandlersRef.current.add(handler)
    console.log('[GlobalEventStream] Total global handlers:', allHandlersRef.current.size)
    return () => {
      console.log('[GlobalEventStream] Unsubscribing from all events', handler)
      console.log('[GlobalEventStream] Unsubscribe stack trace:', new Error().stack?.split('\n').slice(2, 7).join('\n'))
      allHandlersRef.current.delete(handler)
      console.log('[GlobalEventStream] Remaining global handlers:', allHandlersRef.current.size)
    }
  }, [])

  const subscribeToType = useCallback((type: EventType, handler: (event: Event) => void) => {
    console.log('[GlobalEventStream] Subscribing to events of type:', type)
    if (!typeHandlersRef.current.has(type)) {
      typeHandlersRef.current.set(type, new Set())
    }
    typeHandlersRef.current.get(type)!.add(handler)
    console.log('[GlobalEventStream] Total handlers for type', type, ':', typeHandlersRef.current.get(type)!.size)

    return () => {
      console.log('[GlobalEventStream] Unsubscribing from events of type:', type)
      const handlers = typeHandlersRef.current.get(type)
      if (handlers) {
        handlers.delete(handler)
        if (handlers.size === 0) {
          typeHandlersRef.current.delete(type)
          console.log('[GlobalEventStream] Removed all handlers for type:', type)
        } else {
          console.log('[GlobalEventStream] Remaining handlers for type', type, ':', handlers.size)
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
  }, []) // Empty deps to ensure single connection

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
