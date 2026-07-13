import { useCallback, useEffect } from 'react'
import type { ElementInfo } from '../types'

interface UsePreviewMessagesOptions {
  onConsoleMessage: (data: MessageEvent['data']) => void
  onElementClicked?: ((element: ElementInfo | null) => void) | undefined
  onRouteChange: (payload: { path: string }) => void
  onInspectorScriptLoaded: () => void
  onHeartbeat: () => void
  onInspectorAvailability: (payload: { isAvailable: boolean; elementCount?: number }) => void
  onTroubleshootRequest?: ((reason: string) => void) | undefined
  onProxyError: () => void
  onChatToggleRequest?: (() => void) | undefined
}

/**
 * Window `message` listener for the preview iframe's injected script:
 * dispatches each `preview-iframe`-sourced message to the matching handler.
 */
export function usePreviewMessages({
  onConsoleMessage,
  onElementClicked,
  onRouteChange,
  onInspectorScriptLoaded,
  onHeartbeat,
  onInspectorAvailability,
  onTroubleshootRequest,
  onProxyError,
  onChatToggleRequest,
}: UsePreviewMessagesOptions) {
  // Helper: Log debug messages when console is expanded (currently disabled)
  const logDebug = useCallback((_message: string, _data?: unknown) => {
    // Debug logging disabled
  }, [])

  // Handle messages from the injected script
  useEffect(() => {
    const handleMessage = (event: MessageEvent) => {
      const { type, payload, source } = event.data || {}

      logDebug('[Preview] Received message:', {
        type,
        source,
        payload,
        origin: event.origin,
      })

      if (source !== 'preview-iframe') {
        logDebug('[Preview] Ignoring message - wrong source:', source)
        return
      }

      logDebug('[Preview] Processing message:', type)

      switch (type) {
        case 'console':
          onConsoleMessage(event.data)
          break

        case 'ELEMENT_CLICKED':
          if (onElementClicked) {
            onElementClicked(payload)
          }
          break

        case 'NETWORK_REQUEST':
          logDebug('[Preview Network]:', payload)
          break

        case 'SCROLL_POSITION':
          logDebug('[Preview Scroll]:', payload)
          break

        case 'ROUTE_CHANGE':
          onRouteChange(payload)
          break

        case 'INSPECTOR_SCRIPT_LOADED':
          onInspectorScriptLoaded()
          break

        case 'HEARTBEAT':
          onHeartbeat()
          break

        case 'INSPECTOR_AVAILABILITY':
          onInspectorAvailability(payload)
          break

        case 'TROUBLESHOOT_REQUEST':
          onTroubleshootRequest?.(payload?.reason)
          break

        case 'PROXY_ERROR':
          onProxyError()
          break

        case 'TOGGLE_CHAT':
          onChatToggleRequest?.()
          break

        default:
          // Unknown message type
          break
      }
    }

    window.addEventListener('message', handleMessage)
    return () => window.removeEventListener('message', handleMessage)
  }, [
    onConsoleMessage,
    onElementClicked,
    onRouteChange,
    onInspectorScriptLoaded,
    onHeartbeat,
    onInspectorAvailability,
    onTroubleshootRequest,
    onProxyError,
    onChatToggleRequest,
    logDebug,
  ])
}
