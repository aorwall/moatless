import { useCallback, useEffect } from 'react'
import { usePreview } from './use-preview'

export function usePreviewConnection(url: string) {
  const { isConnected, setConnected, setError } = usePreview()

  const checkConnection = useCallback(async () => {
    // Skip connection check if URL is empty or invalid
    if (!url || url.trim() === '') {
      setConnected(false)
      setError(null)
      return
    }

    try {
      // Use a simple fetch with no-cors mode for localhost
      const controller = new AbortController()
      const timeoutId = setTimeout(() => controller.abort(), 5000)

      await fetch(url, {
        method: 'HEAD',
        mode: 'no-cors',
        signal: controller.signal,
      })

      clearTimeout(timeoutId)
      setConnected(true)
      setError(null)
    } catch (error) {
      setConnected(false)
      if (error instanceof Error && error.name === 'AbortError') {
        setError(`Connection timeout: ${url}`)
      } else {
        setError(`Cannot connect to ${url}`)
      }
    }
  }, [url, setConnected, setError])

  useEffect(() => {
    // Initial connection check
    checkConnection()

    // Set up periodic connection checks
    const interval = setInterval(checkConnection, 5000)

    return () => clearInterval(interval)
  }, [checkConnection])

  return {
    isConnected,
    checkConnection,
  }
}
