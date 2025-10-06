import { useCallback, useEffect, useRef, useState } from 'react'

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
  const abortControllerRef = useRef<AbortController | null>(null)

  const handleEvent = useCallback((event: LogStreamEvent) => {
    switch (event.type) {
      case 'chunk':
        if (event.data.text) {
          setLogs((prev) => prev + event.data.text)
        }
        break
      case 'error':
        setError(event.data.error || 'Unknown error occurred')
        break
      case 'complete':
        // Log stream completed
        break
    }
  }, [])

  useEffect(() => {
    if (!taskId) {
      return
    }

    // Clean up any existing connection
    if (abortControllerRef.current) {
      abortControllerRef.current.abort()
      abortControllerRef.current = null
      setIsConnected(false)
    }

    const abortController = new AbortController()
    abortControllerRef.current = abortController

    const baseUrl = getApiBaseUrl()
    const url = `${baseUrl}/api/v1/tasks/${taskId}/server/logs/stream`

    console.log('[useServerLogsStream] Connecting to:', url)

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
        setError(null)
        console.log('[useServerLogsStream] Connected')

        const reader = response.body.getReader()
        const decoder = new TextDecoder()
        let buffer = ''

        try {
          while (true) {
            const { done, value } = await reader.read()

            if (done) {
              console.log('[useServerLogsStream] Stream ended')
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
                  const data = JSON.parse(line) as LogStreamEvent
                  handleEvent(data)
                } catch (err) {
                  console.error(
                    '[useServerLogsStream] Failed to parse line:',
                    err,
                    'Raw line:',
                    line,
                  )
                }
              }
            }
          }
        } catch (err) {
          if (err instanceof Error && err.name === 'AbortError') {
            console.log('[useServerLogsStream] Connection aborted')
            return
          }
          throw err
        }
      })
      .catch((err) => {
        console.error('[useServerLogsStream] Connection error:', err)
        setIsConnected(false)

        // Only set error if connection wasn't manually aborted
        if (!abortController.signal.aborted && err.name !== 'AbortError') {
          setError(err instanceof Error ? err.message : 'Connection failed')
        }
      })

    // Cleanup on unmount or taskId change
    return () => {
      console.log('[useServerLogsStream] Disconnecting...')
      if (abortControllerRef.current) {
        abortControllerRef.current.abort()
        abortControllerRef.current = null
      }
      setIsConnected(false)
    }
  }, [taskId, handleEvent])

  return {
    logs,
    isConnected,
    error,
  }
}
