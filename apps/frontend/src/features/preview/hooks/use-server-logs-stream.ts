import { useCallback, useEffect, useRef, useState } from 'react'

interface LogStreamEvent {
  type: 'chunk' | 'error' | 'complete'
  data: {
    text?: string
    error?: string
    timestamp: string
  }
}

interface UseServerLogsStreamResult {
  logs: string
  isConnected: boolean
  error: string | null
  connect: () => void
  disconnect: () => void
  reconnect: () => void
}

export function useServerLogsStream(taskId: string): UseServerLogsStreamResult {
  const [logs, setLogs] = useState('')
  const [isConnected, setIsConnected] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const eventSourceRef = useRef<EventSource | null>(null)
  const reconnectTimeoutRef = useRef<NodeJS.Timeout | null>(null)
  const shouldReconnectRef = useRef(false)

  const disconnect = useCallback(() => {
    shouldReconnectRef.current = false

    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current)
      reconnectTimeoutRef.current = null
    }

    if (eventSourceRef.current) {
      eventSourceRef.current.close()
      eventSourceRef.current = null
    }

    setIsConnected(false)
  }, [])

  const connect = useCallback(() => {
    if (!taskId || eventSourceRef.current) {
      return // Already connected or no taskId
    }

    shouldReconnectRef.current = true
    setError(null)

    try {
      const eventSource = new EventSource(`/api/v1/tasks/${taskId}/server/logs/stream`)
      eventSourceRef.current = eventSource

      eventSource.onopen = () => {
        setIsConnected(true)
        setError(null)
        console.log('[ServerLogsStream] Connected to server logs stream')
      }

      eventSource.onerror = (event) => {
        console.error('[ServerLogsStream] Connection error:', event)
        setIsConnected(false)

        if (shouldReconnectRef.current && eventSource.readyState === EventSource.CLOSED) {
          // Connection was closed, attempt to reconnect after a delay
          setError('Connection lost, attempting to reconnect...')

          if (reconnectTimeoutRef.current) {
            clearTimeout(reconnectTimeoutRef.current)
          }

          reconnectTimeoutRef.current = setTimeout(() => {
            if (shouldReconnectRef.current) {
              console.log('[ServerLogsStream] Attempting to reconnect...')
              eventSourceRef.current = null
              connect() // Retry connection
            }
          }, 2000) // Reconnect after 2 seconds
        }
      }

      eventSource.addEventListener('chunk', (event) => {
        try {
          const data = JSON.parse(event.data) as LogStreamEvent['data']
          if (data.text) {
            setLogs((prev) => prev + data.text)
          }
        } catch (error) {
          console.error('[ServerLogsStream] Failed to parse chunk event:', error)
        }
      })

      eventSource.addEventListener('error', (event) => {
        try {
          const messageEvent = event as MessageEvent
          const data = JSON.parse(messageEvent.data) as LogStreamEvent['data']
          setError(data.error || 'Unknown streaming error')
          console.error('[ServerLogsStream] Server error:', data.error)
        } catch (error) {
          console.error('[ServerLogsStream] Failed to parse error event:', error)
          setError('Failed to parse server error')
        }
      })

      eventSource.addEventListener('complete', (event) => {
        try {
          const messageEvent = event as MessageEvent
          const data = JSON.parse(messageEvent.data) as LogStreamEvent['data']
          console.log('[ServerLogsStream] Stream completed at:', data.timestamp)
          setIsConnected(false)
          shouldReconnectRef.current = false // Don't reconnect when server completes normally
        } catch (error) {
          console.error('[ServerLogsStream] Failed to parse complete event:', error)
        }
      })
    } catch (error) {
      console.error('[ServerLogsStream] Failed to create EventSource:', error)
      setError(error instanceof Error ? error.message : 'Failed to connect to server logs')
      setIsConnected(false)
    }
  }, [taskId])

  const reconnect = useCallback(() => {
    disconnect()
    // Short delay before reconnecting
    setTimeout(() => {
      connect()
    }, 100)
  }, [disconnect, connect])

  // Auto-connect when taskId changes
  useEffect(() => {
    if (taskId) {
      connect()
    }

    return () => {
      disconnect()
    }
  }, [taskId, connect, disconnect])

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      disconnect()
    }
  }, [disconnect])

  return {
    logs,
    isConnected,
    error,
    connect,
    disconnect,
    reconnect,
  }
}
