import { useEffect, useState } from 'react'

interface UseServerLogsStreamResult {
  logs: string
  isConnected: boolean
  error: string | null
}

interface LogStreamEvent {
  type: 'chunk' | 'error' | 'complete'
  data: {
    text?: string
    error?: string
    timestamp: string
  }
}

// Get API base URL (same logic as client.ts)
const getApiBaseUrl = () => {
  return import.meta.env.PROD ? '' : import.meta.env.VITE_API_URL || 'http://localhost:8000'
}

export function useServerLogsStream(taskId: string | undefined): UseServerLogsStreamResult {
  const [logs, setLogs] = useState('')
  const [isConnected, setIsConnected] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!taskId) {
      return
    }

    const url = `${getApiBaseUrl()}/api/v1/tasks/${taskId}/server/logs/stream`
    const eventSource = new EventSource(url, { withCredentials: true })

    eventSource.onopen = () => {
      setIsConnected(true)
      setError(null)
    }

    eventSource.onmessage = (event) => {
      try {
        const logEvent: LogStreamEvent = JSON.parse(event.data)

        switch (logEvent.type) {
          case 'chunk':
            if (logEvent.data.text) {
              setLogs((prev) => prev + logEvent.data.text)
            }
            break
          case 'error':
            setError(logEvent.data.error || 'Unknown error occurred')
            break
          case 'complete':
            // Log stream completed
            break
        }
      } catch (err) {
        console.error('[useServerLogsStream] Failed to parse log event:', err)
      }
    }

    eventSource.onerror = (err) => {
      console.error('[useServerLogsStream] SSE connection error:', err)
      setIsConnected(false)
      setError('Connection to log stream failed')
      eventSource.close()
    }

    // Cleanup on unmount or taskId change
    return () => {
      eventSource.close()
      setIsConnected(false)
    }
  }, [taskId])

  return {
    logs,
    isConnected,
    error,
  }
}
