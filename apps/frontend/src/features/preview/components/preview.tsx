import { Loader2, Play, Server } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useServerEvents, useServerStatus, useStartServer } from '@/features/preview'
import { Button } from '@/lib/components/ui/button'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { cn } from '@/lib/utils'
import type { ElementInfo, PreviewProps, ScriptHealthStatus } from '../types'
import { InspectorOverlay } from './inspector-overlay'
import { InspectorPanel } from './inspector-panel'
import { LogsPanel } from './logs-panel'
import { PreviewHeader } from './preview-header'

export function Preview({ taskId, onElementSelected, className }: PreviewProps) {
  const iframeRef = useRef<HTMLIFrameElement>(null)
  const heartbeatTimeoutRef = useRef<NodeJS.Timeout | null>(null)
  const [isConsoleExpanded, setIsConsoleExpanded] = useState(false)

  // Fetch server status via query (initial load)
  const { data: serverStatus } = useServerStatus(taskId)

  // Listen for real-time SSE updates
  const startServer = useStartServer()
  const { serverStatus: eventServerStatus, serverError: eventServerError } = useServerEvents(taskId)

  const [path, setPath] = useState('/')
  const [isInspectorActive, setIsInspectorActive] = useState(false)
  const [isLoaded, setIsLoaded] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [selectedElement, setSelectedElement] = useState<ElementInfo | null>(null)
  const [scriptHealthStatus, setScriptHealthStatus] = useState<ScriptHealthStatus>('not-injected')
  const [lastHeartbeat, setLastHeartbeat] = useState<number | null>(null)
  const [isInspectorAvailable, setIsInspectorAvailable] = useState<boolean | null>(null)

  // Use SSE status when available, fall back to query status
  const effectiveServerStatus = eventServerStatus || serverStatus?.status || 'stopped'
  const isConnected = effectiveServerStatus === 'started'
  const isInitializing = effectiveServerStatus === 'starting'
  const isFailed = effectiveServerStatus === 'failed'

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
  }, [lastHeartbeat, scriptHealthStatus])

  // Handle messages from the injected script
  useEffect(() => {
    const handleMessage = (event: MessageEvent) => {
      const { type, payload, source } = event.data || {}

      // Debug: Log all messages to see what we're receiving (only when console is expanded)
      if (isConsoleExpanded) {
        console.log('[Preview] Received message:', { type, source, payload, origin: event.origin })
      }

      if (source !== 'preview-iframe') {
        if (isConsoleExpanded) {
          console.log('[Preview] Ignoring message - wrong source:', source)
        }
        return
      }

      if (isConsoleExpanded) {
        console.log('[Preview] Processing message:', type)
      }

      switch (type) {
        case 'console': {
          // Enhanced console output - could be sent to logs panel
          // Note: Console messages now use flat structure (level, message at top level)
          const { level, message } = event.data
          if (isConsoleExpanded) {
            console.log(`[Preview ${level}]:`, message)
          }
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
          if (isConsoleExpanded) {
            console.log('[Preview Network]:', payload)
          }
          break

        case 'SCROLL_POSITION':
          // Scroll position changed
          if (isConsoleExpanded) {
            console.log('[Preview Scroll]:', payload)
          }
          break

        case 'ROUTE_CHANGE':
          // Route changed in the preview app - update the address bar display only
          if (isConsoleExpanded) {
            console.log('[Preview Route]:', `Route changed to ${payload.path}`)
          }
          setPath(payload.path)
          break

        case 'INSPECTOR_SCRIPT_LOADED':
          if (isConsoleExpanded) {
            console.log('[Preview Script]:', `Preview inspector script loaded v${payload.version}`)
          }
          setScriptHealthStatus('injected-reachable')
          setLastHeartbeat(Date.now())
          break

        case 'HEARTBEAT':
          // Update heartbeat timestamp
          setLastHeartbeat(Date.now())
          break

        case 'INSPECTOR_AVAILABILITY':
          // Update inspector availability status
          // Always log this for debugging (not conditional on isConsoleExpanded)
          console.log('[Preview] INSPECTOR_AVAILABILITY received:', {
            isAvailable: payload?.isAvailable,
            elementCount: payload?.elementCount,
            fullPayload: payload,
          })
          setIsInspectorAvailable(payload.isAvailable)
          console.log('[Preview] Set isInspectorAvailable to:', payload.isAvailable)
          break

        default:
          // Unknown message type
          break
      }
    }

    window.addEventListener('message', handleMessage)
    return () => window.removeEventListener('message', handleMessage)
  }, [onElementSelected, isConsoleExpanded])

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
  // Always load iframe with root path - navigation handled after load
  const baseUrl = serverStatus?.proxyUrl || serverStatus?.url
  const previewUrl = baseUrl ? `${baseUrl}/` : null

  // Track if we need to update iframe src (only for initial load or explicit refresh)
  const [iframeSrcKey, setIframeSrcKey] = useState(0)

  // Navigate within the iframe without full page reload
  const navigatePreview = useCallback((newPath: string) => {
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
      } catch (error) {
        console.error('[Preview] Failed to navigate within iframe:', error)
        // Fallback: force reload by changing iframe src key
        setPath(newPath)
        setIframeSrcKey((prev) => prev + 1)
      }
    } else {
      // No iframe yet, just update path for initial load
      setPath(newPath)
    }
  }, [])

  const toggleInspector = useCallback(() => {
    setIsInspectorActive((prev) => {
      const newValue = !prev
      if (!newValue) {
        setSelectedElement(null)
      }
      return newValue
    })
  }, [])

  const reloadPreview = useCallback(() => {
    // Force reload by incrementing the iframe key
    setIframeSrcKey((prev) => prev + 1)
  }, [])

  const handleIframeLoad = useCallback(() => {
    setIsLoaded(true)
    setError(null)

    // Reset script health status when iframe loads
    // If script is injected, we'll receive INSPECTOR_SCRIPT_LOADED message
    setScriptHealthStatus('not-injected')

    // Don't reset isInspectorAvailable - it will be updated when the script
    // sends the INSPECTOR_AVAILABILITY message. Resetting here causes race conditions.

    // After iframe loads, navigate to the current path if not root
    if (path !== '/' && iframeRef.current?.contentWindow) {
      // Give the app time to initialize before navigating
      setTimeout(() => {
        navigatePreview(path)
      }, 100)
    }

    // Script injection is now handled server-side to avoid cross-origin issues
    // The preview environment service automatically injects the inspector script
    // into the HTML files before serving them
  }, [path, navigatePreview])

  const handleIframeError = useCallback(() => {
    setError('Failed to load preview')
    setIsLoaded(false)
  }, [])

  const handleStartServer = useCallback(() => {
    startServer.mutate(taskId)
  }, [taskId, startServer])

  return (
    <div className={cn('flex flex-col h-full', className)}>
      <PreviewHeader
        path={path}
        onNavigate={navigatePreview}
        isInspectorActive={isInspectorActive}
        onToggleInspector={toggleInspector}
        scriptHealthStatus={scriptHealthStatus}
        lastHeartbeat={lastHeartbeat}
        onReload={reloadPreview}
        isInspectorAvailable={isInspectorAvailable}
      />

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
                key={iframeSrcKey}
                ref={iframeRef}
                src={previewUrl}
                title="Application Preview"
                sandbox="allow-scripts allow-forms allow-popups allow-modals allow-same-origin allow-storage-access-by-user-activation"
                onLoad={handleIframeLoad}
                onError={handleIframeError}
                className={cn('w-full h-full border-none', !isLoaded && 'opacity-0')}
              />

              {isInspectorActive && (
                <InspectorOverlay
                  iframeRef={iframeRef}
                  isActive={isInspectorActive}
                  onElementSelected={(element) => {
                    setSelectedElement(element)
                    onElementSelected?.(element)
                  }}
                />
              )}
            </>
          )}

          {/* Inspector Panel - Shows element details */}
          {isInspectorActive && selectedElement && (
            <InspectorPanel
              className="z-20"
              element={selectedElement}
              onClose={() => setSelectedElement(null)}
            />
          )}
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
