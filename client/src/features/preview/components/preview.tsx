import { useParams } from '@tanstack/react-router'
import { Loader2, Play, RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useStartServer, useStopServer } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { cn } from '@/lib/utils'
import { usePreviewConnection } from '../hooks/use-preview-connection'
import { usePreviewStore } from '../stores/preview-store'
import type { PreviewProps } from '../types'
import { InspectorOverlay } from './inspector-overlay'
import { InspectorPanel } from './inspector-panel'
import { PreviewToolbar } from './preview-toolbar'

export function Preview({ config, serverStatus, onElementSelected, className }: PreviewProps) {
  const iframeRef = useRef<HTMLIFrameElement>(null)
  const params = useParams({ from: '/tasks/$taskId' })
  const startServer = useStartServer()
  const stopServer = useStopServer()
  const [isRestarting, setIsRestarting] = useState(false)

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
  }, [setLoaded, setError])

  const handleIframeError = useCallback(() => {
    setError('Failed to load preview')
    setLoaded(false)
  }, [setError, setLoaded])

  const handleStartServer = useCallback(() => {
    startServer.mutate(params.taskId)
  }, [params.taskId, startServer])

  const handleRestartServer = useCallback(async () => {
    setIsRestarting(true)
    try {
      // First stop the server
      await new Promise<void>((resolve, reject) => {
        stopServer.mutate(params.taskId, {
          onSuccess: () => resolve(),
          onError: (error) => reject(error),
        })
      })

      // Wait a moment for the server to fully stop
      await new Promise((resolve) => setTimeout(resolve, 1000))

      // Then start it again
      await new Promise<void>((resolve, reject) => {
        startServer.mutate(params.taskId, {
          onSuccess: () => resolve(),
          onError: (error) => reject(error),
        })
      })
    } catch (error) {
      console.error('Failed to restart server:', error)
    } finally {
      setIsRestarting(false)
    }
  }, [params.taskId, stopServer, startServer])

  return (
    <div
      className={cn(
        'flex flex-col h-full bg-background border rounded-lg overflow-hidden',
        className,
      )}
    >
      <PreviewToolbar onReload={reloadPreview} />

      <div className="relative flex-1 overflow-hidden">
        {serverStatus && !serverStatus.running && (
          <div className="absolute inset-0 flex items-center justify-center bg-background/80 backdrop-blur-sm z-10">
            <div className="text-center space-y-4">
              <div className="space-y-2">
                <p className="text-muted-foreground">The preview server is not running</p>
                <p className="text-sm text-muted-foreground">Start the server to see the preview</p>
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
                disabled={isRestarting || stopServer.isPending || startServer.isPending}
                size="sm"
              >
                {isRestarting ? (
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
    </div>
  )
}
