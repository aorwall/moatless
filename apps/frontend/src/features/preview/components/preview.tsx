import { useParams } from '@tanstack/react-router'
import { Loader2, Play, Server } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useServerEvents, useStartServer } from '@/features/preview'
import { Button } from '@/lib/components/ui/button'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { PreviewProvider, usePreview } from '../hooks/use-preview'
import type { PreviewProps } from '../types'
import { InspectorOverlay } from './inspector-overlay'
import { InspectorPanel } from './inspector-panel'
import { LogsPanel } from './logs-panel'
import { PreviewHeader } from './preview-header'

function PreviewContent({ config, serverStatus, onElementSelected, className }: PreviewProps) {
  const iframeRef = useRef<HTMLIFrameElement>(null)
  const heartbeatTimeoutRef = useRef<NodeJS.Timeout | null>(null)
  const [isConsoleExpanded, setIsConsoleExpanded] = useState(false)
  const params = useParams({ from: '/tasks/$taskId' })
  const startServer = useStartServer()
  const { serverStatus: eventServerStatus, serverError: eventServerError } = useServerEvents(
    params.taskId,
  )

  const {
    currentPath,
    isInspectorActive,
    isLoaded,
    error,
    selectedElement,
    scriptHealthStatus,
    lastHeartbeat,
    setConfig,
    setLoaded,
    setError,
    setDisplayPath,
    setScriptHealthStatus,
    updateHeartbeat,
  } = usePreview()

  // Use event-driven server status when available, fall back to polling-based status
  const effectiveServerStatus = eventServerStatus || (serverStatus?.running ? 'running' : 'stopped')
  const isConnected = effectiveServerStatus === 'running'
  const isInitializing = effectiveServerStatus === 'initializing'
  const isFailed = effectiveServerStatus === 'failed'

  // Update config when props change
  useEffect(() => {
    setConfig(config)
  }, [config, setConfig])

  // Monitor heartbeat and update script health status
  useEffect(() => {
    // Clear any existing timeout
    if (heartbeatTimeoutRef.current) {
      clearTimeout(heartbeatTimeoutRef.current)
    }

    // If we have a recent heartbeat, set status to reachable
    if (lastHeartbeat && Date.now() - lastHeartbeat < 10000) {
      setScriptHealthStatus('injected-reachable')
    }

    // Set a timeout to mark script as not reachable if no heartbeat in 10 seconds
    heartbeatTimeoutRef.current = setTimeout(() => {
      if (scriptHealthStatus === 'injected-reachable') {
        setScriptHealthStatus('injected-not-reachable')
      }
    }, 10000)

    return () => {
      if (heartbeatTimeoutRef.current) {
        clearTimeout(heartbeatTimeoutRef.current)
      }
    }
  }, [lastHeartbeat, scriptHealthStatus, setScriptHealthStatus])

  // Handle messages from the injected script
  useEffect(() => {
    const handleMessage = (event: MessageEvent) => {
      const { type, payload, source } = event.data || {}

      // Debug: Log all messages to see what we're receiving
      console.log('[Preview] Received message:', { type, source, payload, origin: event.origin })

      if (source !== 'preview-iframe') {
        console.log('[Preview] Ignoring message - wrong source:', source)
        return
      }

      console.log('[Preview] Processing message:', type)

      switch (type) {
        case 'console': {
          // Enhanced console output - could be sent to logs panel
          // Note: Console messages now use flat structure (level, message at top level)
          const { level, message } = event.data
          console.log(`[Preview ${level}]:`, message)
          break
        }

        case 'ELEMENT_CLICKED':
          // Element was clicked in inspector mode
          if (onElementSelected) {
            onElementSelected(payload)
          }
          break

        case 'NETWORK_REQUEST':
          // Network request captured - could be sent to network panel
          console.log('[Preview Network]:', payload)
          break

        case 'SCROLL_POSITION':
          // Scroll position changed
          console.log('[Preview Scroll]:', payload)
          break

        case 'ROUTE_CHANGE':
          // Route changed in the preview app - update the address bar display only
          console.log('[Preview Route]:', `Route changed to ${payload.path}`)
          setDisplayPath(payload.path)
          break

        case 'INSPECTOR_SCRIPT_LOADED':
          console.log('[Preview Script]:', `Preview inspector script loaded v${payload.version}`)
          setScriptHealthStatus('injected-reachable')
          updateHeartbeat()
          break

        case 'HEARTBEAT':
          // Update heartbeat timestamp
          updateHeartbeat()
          break

        default:
          // Unknown message type
          break
      }
    }

    window.addEventListener('message', handleMessage)
    return () => window.removeEventListener('message', handleMessage)
  }, [onElementSelected, setDisplayPath, setScriptHealthStatus, updateHeartbeat])

  // Function to toggle the element selector in the iframe
  const toggleSelector = useCallback((enabled: boolean) => {
    if (iframeRef.current?.contentWindow) {
      iframeRef.current.contentWindow.postMessage(
        {
          type: 'TOGGLE_SELECTOR',
          payload: enabled,
        },
        '*',
      )
    }
  }, [])

  // Sync selector state with inspector active state
  useEffect(() => {
    toggleSelector(isInspectorActive)
  }, [isInspectorActive, toggleSelector])

  // Use proxy URL if available, otherwise fall back to direct URL
  const previewUrl = serverStatus?.proxyUrl
    ? `${serverStatus.proxyUrl}${currentPath}`
    : serverStatus?.url
      ? `${serverStatus.url}${currentPath}`
      : null

  const reloadPreview = useCallback(() => {
    if (iframeRef.current) {
      // Force reload by reassigning the same URL
      const currentSrc = iframeRef.current.src
      iframeRef.current.src = currentSrc
    }
  }, [])

  const handleIframeLoad = useCallback(() => {
    setLoaded(true)
    setError(null)

    // Reset script health status when iframe loads
    // If script is injected, we'll receive INSPECTOR_SCRIPT_LOADED message
    setScriptHealthStatus('not-injected')

    // Script injection is now handled server-side to avoid cross-origin issues
    // The preview environment service automatically injects the inspector script
    // into the HTML files before serving them
  }, [setLoaded, setError, setScriptHealthStatus])

  const handleIframeError = useCallback(() => {
    setError('Failed to load preview')
    setLoaded(false)
  }, [setError, setLoaded])

  const handleStartServer = useCallback(() => {
    startServer.mutate(params.taskId)
  }, [params.taskId, startServer])

  return (
    <div className={cn('flex flex-col h-full', className)}>
      <PreviewHeader onReload={reloadPreview} />

      <div className="flex-1 flex flex-col overflow-hidden bg-background">
        {/* Preview content takes available space */}
        <div className="flex-1 overflow-hidden relative">
          {effectiveServerStatus === 'stopped' && (
            <div className="absolute inset-0 flex items-center justify-center bg-background/80 backdrop-blur-sm z-10">
              <div className="text-center space-y-4">
                <div className="space-y-2">
                  <p className="text-muted-foreground">The preview server is not running</p>
                  <p className="text-sm text-muted-foreground">
                    Start the server to see the preview
                  </p>
                </div>
                <Button onClick={handleStartServer} disabled={startServer.isPending} size="sm">
                  {startServer.isPending ? (
                    <>
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                      Starting Server...
                    </>
                  ) : (
                    <>
                      <Play className="mr-2 h-4 w-4" />
                      Start Server
                    </>
                  )}
                </Button>
              </div>
            </div>
          )}

          {isInitializing && (
            <div className="absolute inset-0 flex items-center justify-center bg-background/80 backdrop-blur-sm z-10">
              <div className="text-center space-y-6">
                <div className="flex flex-col items-center space-y-4">
                  <Server className="h-12 w-12 text-muted-foreground animate-pulse" />
                  <div className="space-y-2">
                    <p className="text-lg font-medium">Server is starting...</p>
                    <p className="text-sm text-muted-foreground">
                      Please wait while the preview server initializes
                    </p>
                  </div>
                </div>

                {/* Skeleton loader for the preview area */}
                <div className="w-full max-w-md mx-auto space-y-3">
                  <Skeleton className="h-4 w-full" />
                  <Skeleton className="h-4 w-3/4" />
                  <Skeleton className="h-4 w-1/2" />
                </div>
              </div>
            </div>
          )}

          {isFailed && (
            <div className="absolute inset-0 flex items-center justify-center bg-background/80 backdrop-blur-sm z-10">
              <div className="text-center space-y-4 max-w-md">
                <div className="space-y-2">
                  <p className="text-destructive font-medium">Server failed to start</p>
                  <p className="text-sm text-muted-foreground">
                    {eventServerError || 'An error occurred while starting the preview server'}
                  </p>
                </div>
                <Button
                  onClick={handleStartServer}
                  disabled={startServer.isPending}
                  size="sm"
                  variant="destructive"
                >
                  {startServer.isPending ? (
                    <>
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                      Retrying...
                    </>
                  ) : (
                    <>
                      <Play className="mr-2 h-4 w-4" />
                      Retry
                    </>
                  )}
                </Button>
              </div>
            </div>
          )}

          {isConnected && !previewUrl && (
            <div className="absolute inset-0 flex items-center justify-center bg-background/80 backdrop-blur-sm z-10">
              <div className="text-center space-y-4">
                <div className="space-y-2">
                  <p className="text-muted-foreground">Preview URL is not available</p>
                  <p className="text-sm text-muted-foreground">
                    Restart the server to initialize the preview
                  </p>
                </div>
                <Button onClick={handleStartServer} disabled={startServer.isPending} size="sm">
                  {startServer.isPending ? (
                    <>
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                      Starting Server...
                    </>
                  ) : (
                    <>
                      <Play className="mr-2 h-4 w-4" />
                      Restart Server
                    </>
                  )}
                </Button>
              </div>
            </div>
          )}

          {error && isConnected && (
            <div className="absolute inset-0 flex items-center justify-center bg-background/80 backdrop-blur-sm z-10">
              <div className="text-center space-y-2">
                <p className="text-destructive">{error}</p>
              </div>
            </div>
          )}

          {isConnected && previewUrl && (
            <>
              <iframe
                ref={iframeRef}
                src={previewUrl}
                title="Application Preview"
                sandbox="allow-scripts allow-forms allow-popups allow-modals allow-same-origin allow-storage-access-by-user-activation"
                onLoad={handleIframeLoad}
                onError={handleIframeError}
                className={cn('w-full h-full border-none', !isLoaded && 'opacity-0')}
              />

              {isInspectorActive && (
                <InspectorOverlay iframeRef={iframeRef} onElementSelected={onElementSelected} />
              )}
            </>
          )}

          {/* Inspector Panel - Shows element details */}
          {isInspectorActive && selectedElement && <InspectorPanel className="z-20" />}
        </div>

        {/* Console panel - always render container, conditionally show content */}
        <div className={isConsoleExpanded ? 'h-64 border-t' : 'h-10 border-t'}>
          <LogsPanel
            className="h-full"
            onToggleExpanded={() => setIsConsoleExpanded(!isConsoleExpanded)}
            isExpanded={isConsoleExpanded}
          />
        </div>
      </div>
    </div>
  )
}

export function Preview(props: PreviewProps) {
  return (
    <PreviewProvider>
      <PreviewContent {...props} />
    </PreviewProvider>
  )
}
