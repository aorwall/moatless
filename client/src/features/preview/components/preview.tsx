import { useRestartServer, useStartServer } from '@/features/preview'
import { Button } from '@/lib/components/ui/button'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { cn } from '@/lib/utils'
import { useParams } from '@tanstack/react-router'
import { Loader2, Play, RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useRef } from 'react'
import { usePreviewConnection } from '../hooks/use-preview-connection'
import { usePreviewStore } from '../stores/preview-store'
import type { PreviewProps } from '../types'
import { InspectorOverlay } from './inspector-overlay'
import { InspectorPanel } from './inspector-panel'
import { LogsPanel } from './logs-panel'
import { PreviewToolbar } from './preview-toolbar'

export function Preview({ config, serverStatus, onElementSelected, className }: PreviewProps) {
  const iframeRef = useRef<HTMLIFrameElement>(null)
  const params = useParams({ from: '/tasks/$taskId' })
  const startServer = useStartServer()
  const restartServer = useRestartServer()

  const {
    currentPath,
    isInspectorActive,
    isLoaded,
    error,
    selectedElement,
    setConfig,
    setLoaded,
    setError,
    setDisplayPath,
  } = usePreviewStore()

  const { isConnected } = usePreviewConnection(config.url)

  // Update config when props change
  useEffect(() => {
    setConfig(config)
  }, [config, setConfig])

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
          break

        default:
          // Unknown message type
          break
      }
    }

    window.addEventListener('message', handleMessage)
    return () => window.removeEventListener('message', handleMessage)
  }, [onElementSelected, setDisplayPath])

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

  // Construct full URL
  const fullUrl = `${config.url}${currentPath}`

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

    // Script injection is now handled server-side to avoid cross-origin issues
    // The preview environment service automatically injects the inspector script
    // into the HTML files before serving them
  }, [setLoaded, setError])

  const handleIframeError = useCallback(() => {
    setError('Failed to load preview')
    setLoaded(false)
  }, [setError, setLoaded])

  const handleStartServer = useCallback(() => {
    startServer.mutate(params.taskId)
  }, [params.taskId, startServer])

  const handleRestartServer = useCallback(() => {
    restartServer.mutate(params.taskId)
  }, [params.taskId, restartServer])

  return (
    <div
      className={cn(
        'flex flex-col h-full bg-background border rounded-lg overflow-hidden',
        className,
      )}
    >
      <PreviewToolbar onReload={reloadPreview} />

      <ResizablePanelGroup direction="vertical" className="flex-1">
        <ResizablePanel defaultSize={75} minSize={30}>
          <div className="relative h-full overflow-hidden">
            {serverStatus && !serverStatus.running && (
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

            {serverStatus?.running && !isConnected && (
              <div className="absolute inset-0 flex items-center justify-center bg-background/80 backdrop-blur-sm z-10">
                <div className="text-center space-y-4">
                  <div className="space-y-2">
                    <p className="text-muted-foreground">
                      Cannot connect to preview server at {config.url}
                    </p>
                    <p className="text-sm text-muted-foreground">
                      The server is running but connection failed
                    </p>
                  </div>
                  <Button
                    onClick={handleRestartServer}
                    disabled={restartServer.isPending}
                    size="sm"
                  >
                    {restartServer.isPending ? (
                      <>
                        <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                        Restarting Server...
                      </>
                    ) : (
                      <>
                        <RefreshCw className="mr-2 h-4 w-4" />
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

            {serverStatus?.running && isConnected && (
              <>
                <iframe
                  ref={iframeRef}
                  src={fullUrl}
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
        </ResizablePanel>

        <ResizableHandle withHandle />

        <ResizablePanel defaultSize={25} minSize={15} maxSize={70}>
          <LogsPanel className="h-full" />
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  )
}
