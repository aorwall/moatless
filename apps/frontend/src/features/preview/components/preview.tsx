import { Loader2, Package, Play, Server } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useServerStatus, useStartServer } from '@/features/preview'
import { Button } from '@/lib/components/ui/button'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
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

  // Fetch server status via query - automatically updated by GlobalEventStreamProvider
  const { data: serverStatus } = useServerStatus(taskId)

  const startServer = useStartServer()

  const [path, setPath] = useState('/')
  const [iframeTargetPath, setIframeTargetPath] = useState('/') // Path to load in iframe (for reloads)
  const [isInspectorActive, setIsInspectorActive] = useState(false)
  const [isLoaded, setIsLoaded] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [selectedElement, setSelectedElement] = useState<ElementInfo | null>(null)
  const [scriptHealthStatus, setScriptHealthStatus] = useState<ScriptHealthStatus>('not-injected')
  const [lastHeartbeat, setLastHeartbeat] = useState<number | null>(null)
  const [isInspectorAvailable, setIsInspectorAvailable] = useState<boolean | null>(null)
  const [consoleLogs, setConsoleLogs] = useState<
    {
      id: string
      timestamp: string
      level: 'log' | 'info' | 'warn' | 'error' | 'debug'
      message: string
      args?: unknown[]
      stack?: string
    }[]
  >([])

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

  // Cleanup when server stops
  useEffect(() => {
    if (serverStatus?.status === 'stopped') {
      setConsoleLogs([])
      setIsInspectorActive(false)
      setSelectedElement(null)
      setIsInspectorAvailable(null)
      setIsLoaded(false)
      setError(null)
      setScriptHealthStatus('not-injected')
      setLastHeartbeat(null)
      // Keep path as-is so server can resume at same path when restarted
    }
  }, [serverStatus?.status])

  // Helper: Handle console message
  const handleConsoleMessage = useCallback((data: MessageEvent['data']) => {
    const { level, message, args, stack } = data

    setConsoleLogs((prev) => [
      ...prev,
      {
        id: `${Date.now()}-${Math.random()}`,
        timestamp: new Date().toISOString(),
        level,
        message,
        args,
        stack,
      },
    ])
  }, [])

  // Helper: Handle route change message
  const handleRouteChange = useCallback((payload: { path: string }) => {
    setPath(payload.path)
  }, [])

  // Helper: Handle inspector script loaded message
  const handleInspectorScriptLoaded = useCallback((_payload: { version: string }) => {
    setScriptHealthStatus('injected-reachable')
    setLastHeartbeat(Date.now())
  }, [])

  // Helper: Handle inspector availability message
  const handleInspectorAvailability = useCallback(
    (payload: { isAvailable: boolean; elementCount?: number }) => {
      setIsInspectorAvailable(payload.isAvailable)
    },
    [],
  )

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
          handleConsoleMessage(event.data)
          break

        case 'ELEMENT_CLICKED':
          if (onElementSelected) {
            onElementSelected(payload)
          }
          break

        case 'NETWORK_REQUEST':
          logDebug('[Preview Network]:', payload)
          break

        case 'SCROLL_POSITION':
          logDebug('[Preview Scroll]:', payload)
          break

        case 'ROUTE_CHANGE':
          handleRouteChange(payload)
          break

        case 'INSPECTOR_SCRIPT_LOADED':
          handleInspectorScriptLoaded(payload)
          break

        case 'HEARTBEAT':
          setLastHeartbeat(Date.now())
          break

        case 'INSPECTOR_AVAILABILITY':
          handleInspectorAvailability(payload)
          break

        default:
          // Unknown message type
          break
      }
    }

    window.addEventListener('message', handleMessage)
    return () => window.removeEventListener('message', handleMessage)
  }, [
    onElementSelected,
    handleConsoleMessage,
    handleRouteChange,
    handleInspectorScriptLoaded,
    handleInspectorAvailability,
    logDebug,
  ])

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

  // URL is automatically updated via optimistic updates in GlobalEventStreamProvider
  // Use iframeTargetPath (not path) to avoid reloading iframe on every navigation
  const baseUrl = serverStatus?.url
  const previewUrl = baseUrl ? `${baseUrl}${iframeTargetPath}` : null

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
      } catch (_error) {
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
    // Update target path to current path before reloading
    setIframeTargetPath(path)
    // Force reload by incrementing the iframe key
    setIframeSrcKey((prev) => prev + 1)
  }, [path])

  const handleIframeLoad = useCallback(() => {
    setIsLoaded(true)
    setError(null)

    // Reset script health status when iframe loads
    // If script is injected, we'll receive INSPECTOR_SCRIPT_LOADED message
    setScriptHealthStatus('not-injected')

    // Don't reset isInspectorAvailable - it will be updated when the script
    // sends the INSPECTOR_AVAILABILITY message. Resetting here causes race conditions.

    // Script injection is now handled server-side to avoid cross-origin issues
    // The preview environment service automatically injects the inspector script
    // into the HTML files before serving them
  }, [])

  const handleIframeError = useCallback(() => {
    setError('Failed to load preview')
    setIsLoaded(false)
  }, [])

  const handleStartServer = useCallback(() => {
    startServer.mutate(taskId)
  }, [taskId, startServer])

  const handleToggleConsole = useCallback(() => {
    setIsConsoleExpanded(!isConsoleExpanded)
  }, [isConsoleExpanded])

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

      <ResizablePanelGroup direction="vertical" className="flex-1 overflow-hidden bg-background">
        {/* Preview content panel */}
        <ResizablePanel className="overflow-hidden relative">
          {(!serverStatus?.status || serverStatus.status === 'stopped') && (
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

          {serverStatus?.status === 'installing' && (
            <div className="absolute inset-0 flex items-center justify-center bg-background/80 backdrop-blur-sm z-10">
              <div className="text-center space-y-6">
                <div className="flex flex-col items-center space-y-4">
                  <Package className="h-12 w-12 text-muted-foreground animate-pulse" />
                  <div className="space-y-2">
                    <p className="text-lg font-medium">Installing dependencies...</p>
                    <p className="text-sm text-muted-foreground">Running setup commands</p>
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

          {serverStatus?.status === 'starting' && (
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

          {serverStatus?.status === 'failed' && (
            <div className="absolute inset-0 flex items-center justify-center bg-background/80 backdrop-blur-sm z-10">
              <div className="text-center space-y-4 max-w-md px-4">
                <div className="space-y-2">
                  <p className="text-destructive font-medium">Server failed to start</p>
                  <p className="text-sm text-muted-foreground">
                    {serverStatus?.error || 'An error occurred while starting the preview server'}
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

          {serverStatus?.status === 'started' && !previewUrl && (
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

          {error && serverStatus?.status === 'started' && (
            <div className="absolute inset-0 flex items-center justify-center bg-background/80 backdrop-blur-sm z-10">
              <div className="text-center space-y-2">
                <p className="text-destructive">{error}</p>
              </div>
            </div>
          )}

          {serverStatus?.status === 'started' && previewUrl && (
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
        </ResizablePanel>

        {/* Resize handle - only show when console is expanded */}
        {isConsoleExpanded && <ResizableHandle withHandle />}

        {/* Console panel - resizable when expanded */}
        {isConsoleExpanded && (
          <ResizablePanel defaultSize={20} minSize={5} maxSize={70}>
            <LogsPanel
              className="h-full"
              onToggleExpanded={handleToggleConsole}
              isExpanded={true}
              consoleLogs={consoleLogs}
            />
          </ResizablePanel>
        )}
      </ResizablePanelGroup>

      {/* Console panel collapsed state - fixed 40px outside the ResizablePanelGroup */}
      {!isConsoleExpanded && (
        <div className="border-t" style={{ height: '40px', flexShrink: 0 }}>
          <LogsPanel
            className="h-full"
            onToggleExpanded={handleToggleConsole}
            isExpanded={false}
            consoleLogs={consoleLogs}
          />
        </div>
      )}
    </div>
  )
}
