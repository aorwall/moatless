import { useCallback, useEffect, useRef, useState } from 'react'

// SSE streaming — api-client can't generate a streaming fetch.
// Path from OpenAPI spec: GET /api/sandbox/v1/tasks/{task_id}/servers/{server_name}/logs
const SERVER_LOGS_PATH = '/api/sandbox/v1/tasks/:taskId/servers/:serverName/logs'

interface UseServerLogsStreamResult {
  logs: string
  isConnected: boolean
  error: string | null
}

interface LogStreamEvent {
  type: 'chunk' | 'error' | 'complete' | 'reset'
  data: {
    text?: string
    error?: string
    timestamp: string
  }
}

// Get API base URL (same logic as client.ts)
const getApiBaseUrl = () => {
  return import.meta.env.PROD ? '' : (import.meta.env.VITE_API_URL ?? '')
}

// Helper: Process a single NDJSON line
function processNDJSONLine(line: string, handleEvent: (event: LogStreamEvent) => void): void {
  if (!line.trim()) {
    return
  }

  try {
    const data = JSON.parse(line) as LogStreamEvent
    handleEvent(data)
  } catch (_err) {
    // Silently ignore malformed NDJSON lines
  }
}

// Helper: Read stream and process NDJSON lines
async function processStream(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  handleEvent: (event: LogStreamEvent) => void,
): Promise<void> {
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
      processNDJSONLine(line, handleEvent)
    }
  }
}

// Helper: Handle successful fetch response
async function handleFetchResponse(
  response: Response,
  handleEvent: (event: LogStreamEvent) => void,
  setIsConnected: (value: boolean) => void,
  setError: (value: string | null) => void,
): Promise<void> {
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}: ${response.statusText}`)
  }

  if (!response.body) {
    throw new Error('No response body')
  }

  setIsConnected(true)
  setError(null)

  const reader = response.body.getReader()

  try {
    await processStream(reader, handleEvent)
  } catch (err) {
    if (err instanceof Error && err.name === 'AbortError') {
      return
    }
    throw err
  }
}

// Helper: Handle fetch error
function handleFetchError(
  err: unknown,
  abortController: AbortController,
  setIsConnected: (value: boolean) => void,
  setError: (value: string | null) => void,
): void {
  setIsConnected(false)

  // Only set error if connection wasn't manually aborted
  if (!abortController.signal.aborted && err instanceof Error && err.name !== 'AbortError') {
    setError(err.message)
  }
}

export function useServerLogsStream(
  taskId: string | undefined,
  serverName: string | undefined,
): UseServerLogsStreamResult {
  const [logs, setLogs] = useState('')
  const [isConnected, setIsConnected] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const abortControllerRef = useRef<AbortController | null>(null)

  const handleEvent = useCallback((event: LogStreamEvent) => {
    switch (event.type) {
      case 'reset':
        // Clear logs when server session resets (new server session starting)
        setLogs('')
        break
      case 'chunk':
        if (event.data.text) {
          setLogs((prev) => prev + event.data.text)
        }
        break
      case 'error':
        setError(event.data.error || 'Unknown error occurred')
        break
      case 'complete':
        // Log stream completed - keep logs visible
        setIsConnected(false)
        break
    }
  }, [])

  useEffect(() => {
    // Only connect when we have both taskId and serverName
    if (!(taskId && serverName)) {
      // Clean up any existing connection
      if (abortControllerRef.current) {
        abortControllerRef.current.abort()
        abortControllerRef.current = null
        setIsConnected(false)
      }
      return
    }

    // Clean up any existing connection before creating new one
    if (abortControllerRef.current) {
      abortControllerRef.current.abort()
      abortControllerRef.current = null
      setIsConnected(false)
    }

    // Reset logs when switching servers
    setLogs('')
    setError(null)

    const abortController = new AbortController()
    abortControllerRef.current = abortController

    const baseUrl = getApiBaseUrl()
    const url = `${baseUrl}${SERVER_LOGS_PATH.replace(':taskId', taskId).replace(':serverName', serverName)}`

    fetch(url, {
      method: 'GET',
      headers: {
        Accept: 'text/plain',
        'Cache-Control': 'no-cache',
      },
      credentials: 'include',
      signal: abortController.signal,
    })
      .then((response) => handleFetchResponse(response, handleEvent, setIsConnected, setError))
      .catch((err) => handleFetchError(err, abortController, setIsConnected, setError))

    // Cleanup on unmount or dependency change
    return () => {
      if (abortControllerRef.current) {
        abortControllerRef.current.abort()
        abortControllerRef.current = null
      }
      setIsConnected(false)
    }
  }, [taskId, serverName, handleEvent])

  return {
    logs,
    isConnected,
    error,
  }
}
