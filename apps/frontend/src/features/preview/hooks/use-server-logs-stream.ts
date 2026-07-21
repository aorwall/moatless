import { useEffect, useState } from 'react'

// SSE streaming — api-client can't generate a streaming fetch.
// Path from OpenAPI spec: GET /api/sandbox/v1/tasks/{task_id}/servers/{server_name}/logs
const SERVER_LOGS_PATH = '/api/sandbox/v1/tasks/:taskId/servers/:serverName/logs'

interface UseServerLogsStreamResult {
  logs: string
  isConnected: boolean
  error: string | null
}

interface ServerLogsStreamState extends UseServerLogsStreamResult {
  streamKey: string | null
}

const EMPTY_STREAM_STATE: UseServerLogsStreamResult = {
  logs: '',
  isConnected: false,
  error: null,
}

// Get API base URL (same logic as client.ts)
const getApiBaseUrl = () => {
  return import.meta.env.PROD ? '' : (import.meta.env.VITE_API_URL ?? '')
}

export function useServerLogsStream(
  taskId: string | undefined,
  serverName: string | undefined,
): UseServerLogsStreamResult {
  const streamKey = taskId && serverName ? `${taskId}:${serverName}` : null
  const [streamState, setStreamState] = useState<ServerLogsStreamState>({
    streamKey: null,
    ...EMPTY_STREAM_STATE,
  })

  useEffect(() => {
    if (!(taskId && serverName && streamKey)) {
      return
    }

    const baseUrl = getApiBaseUrl()
    const url = `${baseUrl}${SERVER_LOGS_PATH.replace(':taskId', taskId).replace(':serverName', serverName)}`

    const eventSource = new EventSource(url, { withCredentials: true })

    eventSource.onopen = () => {
      setStreamState({ streamKey, logs: '', isConnected: true, error: null })
    }

    eventSource.onmessage = (event) => {
      setStreamState((current) => ({
        streamKey,
        logs: `${current.streamKey === streamKey ? current.logs : ''}${event.data}\n`,
        isConnected: true,
        error: null,
      }))
    }

    eventSource.onerror = () => {
      // EventSource automatically reconnects on transient errors.
      // Only mark disconnected — don't set error for transient issues.
      setStreamState((current) =>
        current.streamKey === streamKey ? { ...current, isConnected: false } : current,
      )
    }

    return () => eventSource.close()
  }, [taskId, serverName, streamKey])

  return streamState.streamKey === streamKey ? streamState : EMPTY_STREAM_STATE
}
