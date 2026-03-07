import { useEffect, useRef, useState } from 'react'

// SSE streaming — api-client can't generate a streaming fetch.
// Path from OpenAPI spec: GET /api/sandbox/v1/tasks/{task_id}/servers/{server_name}/logs
const SERVER_LOGS_PATH = '/api/sandbox/v1/tasks/:taskId/servers/:serverName/logs'

interface UseServerLogsStreamResult {
  logs: string
  isConnected: boolean
  error: string | null
}

// Get API base URL (same logic as client.ts)
const getApiBaseUrl = () => {
  return import.meta.env.PROD ? '' : (import.meta.env.VITE_API_URL ?? '')
}

export function useServerLogsStream(
  taskId: string | undefined,
  serverName: string | undefined,
): UseServerLogsStreamResult {
  const [logs, setLogs] = useState('')
  const [isConnected, setIsConnected] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const eventSourceRef = useRef<EventSource | null>(null)

  useEffect(() => {
    if (!(taskId && serverName)) {
      if (eventSourceRef.current) {
        eventSourceRef.current.close()
        eventSourceRef.current = null
        setIsConnected(false)
      }
      return
    }

    // Clean up existing connection
    if (eventSourceRef.current) {
      eventSourceRef.current.close()
      eventSourceRef.current = null
    }

    setLogs('')
    setError(null)

    const baseUrl = getApiBaseUrl()
    const url = `${baseUrl}${SERVER_LOGS_PATH.replace(':taskId', taskId).replace(':serverName', serverName)}`

    const eventSource = new EventSource(url, { withCredentials: true })
    eventSourceRef.current = eventSource

    eventSource.onopen = () => {
      setIsConnected(true)
      setError(null)
    }

    eventSource.onmessage = (event) => {
      setLogs((prev) => `${prev + event.data}\n`)
    }

    eventSource.onerror = () => {
      // EventSource automatically reconnects on transient errors.
      // Only mark disconnected — don't set error for transient issues.
      setIsConnected(false)
    }

    return () => {
      eventSource.close()
      eventSourceRef.current = null
      setIsConnected(false)
    }
  }, [taskId, serverName])

  return { logs, isConnected, error }
}
