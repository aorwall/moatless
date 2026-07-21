import { type RefObject, useEffect, useRef, useState } from 'react'

interface UsePreviewNavigationOptions {
  iframeRef: RefObject<HTMLIFrameElement | null>
  /** Current preview path from URL state. */
  path: string
  /** Update the preview path in URL state. */
  setPath: (path: string) => void
  /** Status of the active server ('started' triggers an auto-reload). */
  serverStatus: string | undefined
}

/**
 * Iframe URL/reload lifecycle: which path the iframe is loaded at
 * (`iframeTargetPath`, decoupled from the live `path` so in-app navigation
 * doesn't reload the frame), the reload key, soft navigation via postMessage,
 * and the auto-reload when the server transitions to 'started'.
 */
export function usePreviewNavigation({
  iframeRef,
  path,
  setPath,
  serverStatus,
}: UsePreviewNavigationOptions) {
  const [iframeTargetPath, setIframeTargetPath] = useState(path) // Path to load in iframe (for reloads)
  // Track if we need to update iframe src (only for initial load or explicit refresh)
  const [iframeSrcKey, setIframeSrcKey] = useState(0)
  const previousStatusRef = useRef<string | undefined>(undefined)
  const reloadTimeoutRef = useRef<NodeJS.Timeout | null>(null)

  // Auto-reload iframe when server transitions to 'started' state
  useEffect(() => {
    const currentStatus = serverStatus
    const previousStatus = previousStatusRef.current

    // Detect transition to 'started' from any other state
    if (
      currentStatus === 'started' &&
      previousStatus !== 'started' &&
      previousStatus !== undefined
    ) {
      // Clear any existing timeout
      if (reloadTimeoutRef.current) {
        clearTimeout(reloadTimeoutRef.current)
      }

      // Set timeout to reload after 500ms (ensures server is fully ready)
      reloadTimeoutRef.current = setTimeout(() => {
        // Update target path to current path before reloading
        setIframeTargetPath(path)
        // Force reload by incrementing the iframe key
        setIframeSrcKey((prev) => prev + 1)
      }, 500)
    }

    // Update previous status
    previousStatusRef.current = currentStatus

    return () => {
      if (reloadTimeoutRef.current) {
        clearTimeout(reloadTimeoutRef.current)
      }
    }
  }, [serverStatus, path])

  // Navigate within the iframe without full page reload
  const navigatePreview = (newPath: string) => {
    if (iframeRef.current?.contentWindow) {
      try {
        // Update path state for UI
        setPath(newPath)

        // Use postMessage to request navigation (works cross-origin)
        // The injected script will handle the actual navigation
        iframeRef.current.contentWindow.postMessage(
          {
            type: 'NAVIGATE',
            payload: { path: newPath },
          },
          '*',
        )
      } catch (_error) {
        // Fallback: force reload by changing iframe src key
        setPath(newPath)
        setIframeSrcKey((prev) => prev + 1)
      }
    } else {
      // No iframe yet, just update path for initial load
      setPath(newPath)
    }
  }

  /** Hard-reload the iframe at the current path. */
  const reloadIframe = () => {
    // Update target path to current path before reloading
    setIframeTargetPath(path)
    // Force reload by incrementing the iframe key
    setIframeSrcKey((prev) => prev + 1)
  }

  return { iframeTargetPath, setIframeTargetPath, iframeSrcKey, navigatePreview, reloadIframe }
}
