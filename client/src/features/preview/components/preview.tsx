import { useParams } from '@tanstack/react-router'
import { Loader2, Play, RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useRef } from 'react'
import { useRestartServer, useStartServer } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { cn } from '@/lib/utils'
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
  } = usePreviewStore()

  const { isConnected } = usePreviewConnection(config.url)

  // Update config when props change
  useEffect(() => {
    setConfig(config)
  }, [config, setConfig])

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

    // Inject console interceptor into iframe
    if (iframeRef.current?.contentWindow) {
      try {
        const script = iframeRef.current.contentWindow.document.createElement('script')
        script.textContent = `
          (function() {
            // Store original console methods
            const originalConsole = {
              log: console.log,
              error: console.error,
              warn: console.warn,
              info: console.info,
              debug: console.debug
            };
            
            // Helper to send console messages to parent
            function sendConsoleMessage(level, args) {
              try {
                window.parent.postMessage({
                  type: 'console',
                  source: 'preview-iframe',
                  level: level,
                  message: args.map(arg => {
                    if (typeof arg === 'object') {
                      try {
                        return JSON.stringify(arg, null, 2);
                      } catch (e) {
                        return String(arg);
                      }
                    }
                    return String(arg);
                  }).join(' '),
                  args: args
                }, '*');
              } catch (e) {
                // Silently fail if postMessage fails
              }
            }
            
            // Override console methods
            console.log = function(...args) {
              originalConsole.log.apply(console, args);
              sendConsoleMessage('log', args);
            };
            
            console.error = function(...args) {
              originalConsole.error.apply(console, args);
              sendConsoleMessage('error', args);
            };
            
            console.warn = function(...args) {
              originalConsole.warn.apply(console, args);
              sendConsoleMessage('warn', args);
            };
            
            console.info = function(...args) {
              originalConsole.info.apply(console, args);
              sendConsoleMessage('info', args);
            };
            
            console.debug = function(...args) {
              originalConsole.debug.apply(console, args);
              sendConsoleMessage('debug', args);
            };
            
            // Also capture uncaught errors
            window.addEventListener('error', function(event) {
              sendConsoleMessage('error', [event.message, 'at', event.filename + ':' + event.lineno + ':' + event.colno]);
            });
            
            window.addEventListener('unhandledrejection', function(event) {
              sendConsoleMessage('error', ['Unhandled Promise Rejection:', event.reason]);
            });
          })();
        `
        iframeRef.current.contentWindow.document.head.appendChild(script)
      } catch (e) {
        // Silently fail if we can't inject the script (e.g., cross-origin)
        console.warn('Could not inject console interceptor into iframe:', e)
      }
    }
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
