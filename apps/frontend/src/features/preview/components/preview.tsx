import { AlertCircle, Loader2, MessageSquarePlus, Server, XCircle } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { FeedbackDialog } from '@/features/feedback/components/feedback-dialog'
import { useSandboxRetry } from '@/lib/api/task-sandbox-queries'
import { Button } from '@/lib/components/ui/button'
import { StackedConsoleSplit } from '@/lib/layout/stacked-console-split'
import { cn } from '@/lib/utils'
import { usePreviewDevice, usePreviewPath } from '../hooks/use-preview-view'
import type { PreviewProps, ScriptHealthStatus } from '../types'
import { DeviceFrame } from './device-frame'
import { InspectorOverlay } from './inspector-overlay'
import { LogsPanel } from './logs-panel'
import { PreviewHeader } from './preview-header'
import { SandboxStatusOverlay } from './sandbox-status-overlay'

function getResourceColor(
  cpuPercent: number | null | undefined,
  loopMs: number | null | undefined,
): string {
  const cpu = cpuPercent ?? 0
  const loop = loopMs ?? 0
  if (cpu > 80 || loop > 1000) {
    return 'text-red-500'
  }
  if (cpu > 50 || loop > 100) {
    return 'text-yellow-500'
  }
  return 'text-muted-foreground'
}

function formatMemory(mb: number): string {
  if (mb >= 1024) {
    return `${(mb / 1024).toFixed(1)} GB`
  }
  return `${Math.round(mb)} MB`
}

function ProxyErrorOverlay({
  taskId,
  onReload,
  serverName,
  serverLabel,
  previewUrl,
}: {
  taskId?: string | undefined
  onReload: () => void
  serverName?: string | undefined
  serverLabel?: string | undefined
  previewUrl?: string | undefined
}) {
  const [feedbackOpen, setFeedbackOpen] = useState(false)
  const errorMessage =
    'Could not connect to the preview server. It may have crashed or be listening on a different port.'
  return (
    <div className="absolute inset-0 flex items-center justify-center z-10 bg-background/80 backdrop-blur-sm">
      <div className="flex flex-col items-center justify-center text-center space-y-4 p-8">
        <XCircle className="h-12 w-12 text-destructive" />
        <h3 className="text-lg font-semibold">Connection Failed</h3>
        <p className="text-sm text-muted-foreground max-w-md">{errorMessage}</p>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={onReload}>
            Retry
          </Button>
          {taskId && (
            <Button variant="outline" size="sm" onClick={() => setFeedbackOpen(true)}>
              <MessageSquarePlus className="h-4 w-4 mr-2" />
              Give feedback
            </Button>
          )}
        </div>
      </div>
      {taskId && (
        <FeedbackDialog
          open={feedbackOpen}
          onOpenChange={setFeedbackOpen}
          taskId={taskId}
          surface="preview_failure"
          metadata={{
            preview_error: errorMessage,
            server_name: serverName,
            server_label: serverLabel,
            preview_url: previewUrl,
          }}
        />
      )}
    </div>
  )
}

function ServerStatusView({
  server,
}: {
  server: { status: string; label: string; name: string; error?: string | null }
}) {
  return (
    <div className="flex flex-col items-center justify-center h-full text-center space-y-4 p-8">
      {(server.status === 'starting' || server.status === 'installing') && (
        <>
          <Loader2 className="h-12 w-12 text-muted-foreground animate-spin" />
          <h3 className="text-lg font-semibold">
            {server.status === 'installing' ? 'Installing Dependencies' : 'Server Starting'}
          </h3>
          <p className="text-sm text-muted-foreground max-w-md">
            Please wait while {server.label || 'the server'} starts...
          </p>
        </>
      )}
      {server.status === 'failed' && (
        <>
          <XCircle className="h-12 w-12 text-destructive" />
          <h3 className="text-lg font-semibold">Server Failed</h3>
          <p className="text-sm text-muted-foreground max-w-md">
            {server.error || 'The server encountered an error'}
          </p>
        </>
      )}
      {server.status === 'stopped' && (
        <>
          <AlertCircle className="h-12 w-12 text-orange-500" />
          <h3 className="text-lg font-semibold">Server Stopped</h3>
          <p className="text-sm text-muted-foreground max-w-md">
            {server.label || 'The server'} is not currently running
          </p>
        </>
      )}
    </div>
  )
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Preview is a complex component managing iframe lifecycle, inspector, and console state
export function Preview({
  taskId,
  environmentStatus,
  environmentError,
  onElementSelected,
  onToggleElementContext,
  onNavigationReady,
  onTroubleshootRequest,
  onChatToggleRequest,
  servers,
  activeServer,
  view,
  onViewChange,
  publish,
  resources,
  sandboxEvents,
  hideHeader = false,
  hideConsole = false,
  className,
}: PreviewProps) {
  // Hook for retrying/starting sandbox
  const retryMutation = useSandboxRetry(taskId)

  // Determine if sandbox is running
  const isSandboxRunning = environmentStatus === 'ready'
  const iframeRef = useRef<HTMLIFrameElement>(null)
  const heartbeatTimeoutRef = useRef<NodeJS.Timeout | null>(null)
  const previousStatusRef = useRef<string | undefined>(undefined)
  const reloadTimeoutRef = useRef<NodeJS.Timeout | null>(null)
  // null = follow the auto-expand default; true/false = user's explicit choice.
  const [consoleExpandedOverride, setConsoleExpandedOverride] = useState<boolean | null>(null)

  // Find the active server entry from the servers array
  const activeServerEntry = useMemo(() => {
    if (servers?.length === 0 || !servers) {
      return
    }
    if (activeServer) {
      return servers.find((s) => s.name === activeServer)
    }
    // Default to the first server
    return servers[0]
  }, [servers, activeServer])

  // Derive server status and URL from the active server entry
  const serverStatus = useMemo(() => {
    if (!activeServerEntry) {
      return
    }
    return {
      status: activeServerEntry.status,
      url: activeServerEntry.url,
      error: activeServerEntry.error,
    }
  }, [activeServerEntry])

  // Auto-expand the console (Events tab) whenever the preview isn't live: the
  // sandbox is still coming up, or the active server hasn't started yet. The
  // user's explicit toggle (override) always wins over this default.
  const shouldAutoExpandConsole = !isSandboxRunning || serverStatus?.status !== 'started'
  const isConsoleExpanded = consoleExpandedOverride ?? shouldAutoExpandConsole

  // Get capabilities from active server
  const capabilities = activeServerEntry?.capabilities

  // Get preview path from URL state
  const { previewPath: path, setPreviewPath: setPath } = usePreviewPath()

  // Get device mode from URL state
  const { device: deviceMode, setDevice } = usePreviewDevice()
  const [iframeTargetPath, setIframeTargetPath] = useState(path) // Path to load in iframe (for reloads)
  const [isInspectorActive, setIsInspectorActive] = useState(false)
  const [isLoaded, setIsLoaded] = useState(false)
  const [scriptHealthStatus, setScriptHealthStatus] = useState<ScriptHealthStatus>('not-injected')
  const [lastHeartbeat, setLastHeartbeat] = useState<number | null>(null)
  const [isInspectorAvailable, setIsInspectorAvailable] = useState<boolean | null>(null)
  const [isPendingInspectorActivation, setIsPendingInspectorActivation] = useState(false)
  const [isInspectorDialogOpen, setIsInspectorDialogOpen] = useState(false)
  const [selectedComponentId, setSelectedComponentId] = useState<string | null>(null)
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
  const [hasProxyError, setHasProxyError] = useState(false)

  // Reset state when active server changes
  // biome-ignore lint/correctness/useExhaustiveDependencies: activeServer is the trigger for resetting state
  useEffect(() => {
    setIsLoaded(false)
    setIsInspectorActive(false)
    setIsInspectorAvailable(null)
    setScriptHealthStatus('not-injected')
    setLastHeartbeat(null)
    setSelectedComponentId(null)
    setConsoleLogs([])
    setHasProxyError(false)
    setPath('/')
    setIframeTargetPath('/')
    setDevice('desktop')
    // Re-apply the auto-expand default for the newly selected server.
    setConsoleExpandedOverride(null)
  }, [activeServer])

  // Monitor heartbeat and update script health status
  useEffect(() => {
    // Clear any existing timeout
    if (heartbeatTimeoutRef.current) {
      clearTimeout(heartbeatTimeoutRef.current)
    }

    // If we have a recent heartbeat, set status to reachable
    if (lastHeartbeat && Date.now() - lastHeartbeat < 10_000) {
      setScriptHealthStatus('injected-reachable')
    }

    // Set a timeout to mark script as not reachable if no heartbeat in 10 seconds
    heartbeatTimeoutRef.current = setTimeout(() => {
      if (scriptHealthStatus === 'injected-reachable') {
        setScriptHealthStatus('injected-not-reachable')
      }
    }, 10_000)

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
      setIsInspectorAvailable(null)
      setIsLoaded(false)
      setScriptHealthStatus('not-injected')
      setLastHeartbeat(null)
      setSelectedComponentId(null)
      // Keep path as-is so server can resume at same path when restarted
    }
  }, [serverStatus?.status])

  // Clear proxy error when server becomes ready/started
  useEffect(() => {
    if (serverStatus?.status === 'started') {
      setHasProxyError(false)
    }
  }, [serverStatus?.status])

  // Auto-reload iframe when server transitions to 'started' state
  useEffect(() => {
    const currentStatus = serverStatus?.status
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
  }, [serverStatus?.status, path])

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
  const handleRouteChange = useCallback(
    (payload: { path: string }) => {
      setPath(payload.path)
    },
    [setPath],
  )

  // Helper: Handle inspector script loaded message
  const handleInspectorScriptLoaded = useCallback((_payload: { version: string }) => {
    setScriptHealthStatus('injected-reachable')
    setLastHeartbeat(Date.now())
  }, [])

  // Helper: Handle inspector availability message
  const handleInspectorAvailability = useCallback(
    (payload: { isAvailable: boolean; elementCount?: number }) => {
      setIsInspectorAvailable(payload.isAvailable)

      // If user was waiting for availability check, auto-enable or show dialog
      if (isPendingInspectorActivation) {
        setIsPendingInspectorActivation(false)

        if (payload.isAvailable) {
          // Auto-enable inspector
          setIsInspectorActive(true)
        } else {
          // Show unavailable dialog
          setIsInspectorDialogOpen(true)
        }
      }
    },
    [isPendingInspectorActivation],
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

        case 'TROUBLESHOOT_REQUEST':
          onTroubleshootRequest?.(payload?.reason)
          break

        case 'PROXY_ERROR':
          setHasProxyError(true)
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
    onElementSelected,
    onTroubleshootRequest,
    onChatToggleRequest,
    handleConsoleMessage,
    handleRouteChange,
    handleInspectorScriptLoaded,
    handleInspectorAvailability,
    logDebug,
  ])

  // Function to request availability check from iframe
  const requestAvailabilityCheck = useCallback(() => {
    if (iframeRef.current?.contentWindow) {
      setIsPendingInspectorActivation(true)
      iframeRef.current.contentWindow.postMessage(
        {
          type: 'CHECK_AVAILABILITY_REQUEST',
        },
        '*',
      )
    }
  }, [])

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

  // Function to clear all selections
  const clearSelection = useCallback(() => {
    if (iframeRef.current?.contentWindow) {
      iframeRef.current.contentWindow.postMessage(
        {
          type: 'CLEAR_SELECTION',
        },
        '*',
      )
    }
  }, [])

  // Function to add element to selection overlay
  const addToSelection = useCallback((componentId: string) => {
    if (iframeRef.current?.contentWindow) {
      iframeRef.current.contentWindow.postMessage(
        {
          type: 'ADD_TO_SELECTION',
          payload: { componentId },
        },
        '*',
      )
    }
  }, [])

  // Function to remove element from selection overlay
  const removeFromSelection = useCallback((componentId: string) => {
    if (iframeRef.current?.contentWindow) {
      iframeRef.current.contentWindow.postMessage(
        {
          type: 'REMOVE_FROM_SELECTION',
          payload: { componentId },
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
  const navigatePreview = useCallback(
    (newPath: string) => {
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
    },
    [setPath],
  )

  // Expose navigation function to parent component
  useEffect(() => {
    if (onNavigationReady) {
      onNavigationReady(navigatePreview)
    }
  }, [onNavigationReady, navigatePreview])

  const toggleInspector = useCallback(() => {
    setIsInspectorActive((prev) => {
      // If disabling inspector, clear selection
      if (prev) {
        setSelectedComponentId(null)
      }
      return !prev
    })
  }, [])

  const reloadPreview = useCallback(() => {
    // Disable inspector on reload
    setIsInspectorActive(false)
    setSelectedComponentId(null)

    // Update target path to current path before reloading
    setIframeTargetPath(path)
    // Force reload by incrementing the iframe key
    setIframeSrcKey((prev) => prev + 1)
  }, [path])

  const handleIframeLoad = useCallback(() => {
    setHasProxyError(false)
    setIsLoaded(true)

    // Reset script health status when iframe loads
    // If script is injected, we'll receive INSPECTOR_SCRIPT_LOADED message
    setScriptHealthStatus('not-injected')

    // Don't reset isInspectorAvailable - it will be updated when the script
    // sends the INSPECTOR_AVAILABILITY message. Resetting here causes race conditions.

    // Script injection is now handled server-side to avoid cross-origin issues
    // The preview environment service automatically injects the inspector script
    // into the HTML files before serving them
  }, [])

  const handleToggleConsole = useCallback(() => {
    setConsoleExpandedOverride(!isConsoleExpanded)
  }, [isConsoleExpanded])

  // Determine if we should show console (only for servers with console capability)
  const showConsole = capabilities?.console ?? false

  const renderConsole = (isExpanded: boolean) => (
    <LogsPanel
      className="h-full"
      onToggleExpanded={handleToggleConsole}
      isExpanded={isExpanded}
      consoleLogs={consoleLogs}
      serverName={activeServerEntry?.name}
      showConsoleTab={showConsole}
      sandboxEvents={sandboxEvents}
    />
  )

  return (
    <div className={cn('flex flex-col h-full', className)}>
      {!hideHeader && (
        <PreviewHeader
          path={path}
          onNavigate={navigatePreview}
          isInspectorActive={isInspectorActive}
          onToggleInspector={toggleInspector}
          onRequestAvailabilityCheck={requestAvailabilityCheck}
          onReload={reloadPreview}
          isInspectorAvailable={isInspectorAvailable}
          isInspectorDialogOpen={isInspectorDialogOpen}
          onInspectorDialogOpenChange={setIsInspectorDialogOpen}
          serverUrl={serverStatus?.url ?? undefined}
          servers={servers}
          view={view}
          onViewChange={onViewChange}
          capabilities={capabilities}
          isSandboxRunning={isSandboxRunning}
          publish={publish}
        />
      )}

      <StackedConsoleSplit
        isConsoleExpanded={isConsoleExpanded}
        hideConsole={hideConsole}
        renderConsole={renderConsole}
        groupClassName="bg-background"
        contentClassName="relative"
        footer={
          resources && isSandboxRunning ? (
            <div
              className={cn(
                'h-6 flex-shrink-0 border-t px-3 flex items-center gap-3 text-xs font-mono tabular-nums',
                getResourceColor(resources.cpuUsagePercent, resources.eventLoopLatencyMs),
              )}
            >
              {resources.cpuUsagePercent !== null && <span>CPU {resources.cpuUsagePercent}%</span>}
              <span>
                Mem {formatMemory(resources.memoryUsageMb)}
                {resources.memoryLimitMb != null ? `/${formatMemory(resources.memoryLimitMb)}` : ''}
              </span>
              <span>Loop {Math.round(resources.eventLoopLatencyMs)}ms</span>
            </div>
          ) : null
        }
      >
        {/* Preview content panel */}
        {/* Sandbox not running overlay */}
        {!isSandboxRunning && environmentStatus && (
          <SandboxStatusOverlay
            environmentStatus={environmentStatus}
            environmentError={environmentError}
            onRetry={() => retryMutation.mutate()}
            isRetrying={retryMutation.isPending}
            retryError={retryMutation.error}
          />
        )}

        {/* No servers configured */}
        {servers && servers.length === 0 && isSandboxRunning && (
          <div className="absolute inset-0 flex items-center justify-center z-10">
            <div className="text-center space-y-2">
              <Server className="h-8 w-8 text-muted-foreground mx-auto" />
              <p className="text-sm text-muted-foreground">No servers configured</p>
            </div>
          </div>
        )}

        {/* Server not started — show status (events surface in the console below) */}
        {isSandboxRunning && activeServerEntry && activeServerEntry.status !== 'started' && (
          <ServerStatusView server={activeServerEntry} />
        )}

        {/* Proxy error overlay — only when server is started */}
        {hasProxyError && isSandboxRunning && serverStatus?.status === 'started' && (
          <ProxyErrorOverlay
            taskId={taskId}
            onReload={reloadPreview}
            serverName={activeServerEntry?.name}
            serverLabel={activeServerEntry?.label}
            previewUrl={previewUrl ?? undefined}
          />
        )}

        {/* Render iframe only when server is started */}
        {previewUrl && serverStatus?.status === 'started' && (
          <DeviceFrame mode={capabilities?.viewport ? deviceMode : 'desktop'}>
            <iframe
              key={iframeSrcKey}
              ref={iframeRef}
              src={previewUrl}
              title="Application Preview"
              sandbox="allow-scripts allow-forms allow-popups allow-modals allow-same-origin allow-storage-access-by-user-activation"
              onLoad={handleIframeLoad}
              className={cn('w-full h-full border-none', !isLoaded && 'opacity-0')}
            />

            {isInspectorActive && (
              <InspectorOverlay
                iframeRef={iframeRef}
                isActive={isInspectorActive}
                onElementSelected={(element) => {
                  // Transform iframe-relative coordinates to parent-window coordinates
                  let transformedElement = element
                  if (element.elementRect && iframeRef.current) {
                    const iframeRect = iframeRef.current.getBoundingClientRect()
                    transformedElement = {
                      ...element,
                      elementRect: {
                        top: element.elementRect.top + iframeRect.top,
                        left: element.elementRect.left + iframeRect.left,
                        width: element.elementRect.width,
                        height: element.elementRect.height,
                        bottom: element.elementRect.bottom + iframeRect.top,
                        right: element.elementRect.right + iframeRect.left,
                      },
                    }
                  }

                  // Check if clicking the same element again (to toggle it off)
                  const isSameElement =
                    selectedComponentId && transformedElement.componentId === selectedComponentId

                  if (isSameElement) {
                    // Clicking same element again - remove selection overlay and from context
                    removeFromSelection(transformedElement.componentId)
                    setSelectedComponentId(null)
                  } else {
                    // Different element - clear old selection and add new one
                    clearSelection()
                    addToSelection(transformedElement.componentId)
                    setSelectedComponentId(transformedElement.componentId)
                  }

                  // Notify parent of selection (optional callback)
                  onElementSelected?.(transformedElement)
                  // Automatically toggle element in context
                  onToggleElementContext?.(transformedElement)
                }}
              />
            )}
          </DeviceFrame>
        )}
      </StackedConsoleSplit>
    </div>
  )
}
