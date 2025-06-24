import { useCallback, useEffect, useRef } from 'react'
import { cn } from '@/lib/utils'
import { usePreviewConnection } from '../hooks/use-preview-connection'
import { usePreviewStore } from '../stores/preview-store'
import type { PreviewProps } from '../types'
import { InspectorOverlay } from './inspector-overlay'
import { InspectorPanel } from './inspector-panel'
import { PreviewToolbar } from './preview-toolbar'

export function Preview({ config, onElementSelected, className }: PreviewProps) {
  const iframeRef = useRef<HTMLIFrameElement>(null)

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
      iframeRef.current.src = iframeRef.current.src
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

  return (
    <div
      className={cn(
        'flex flex-col h-full bg-background border rounded-lg overflow-hidden',
        className,
      )}
    >
      <PreviewToolbar onReload={reloadPreview} />

      <div className="relative flex-1 overflow-hidden">
        {!isConnected && (
          <div className="absolute inset-0 flex items-center justify-center bg-background/80 backdrop-blur-sm z-10">
            <div className="text-center space-y-2">
              <p className="text-muted-foreground">
                Cannot connect to preview server at {config.url}
              </p>
              <p className="text-sm text-muted-foreground">
                Make sure your development server is running
              </p>
            </div>
          </div>
        )}

        {error && isConnected && (
          <div className="absolute inset-0 flex items-center justify-center bg-background/80 backdrop-blur-sm z-10">
            <div className="text-center space-y-2">
              <p className="text-destructive">{error}</p>
              <button onClick={reloadPreview} className="text-sm text-primary hover:underline">
                Try again
              </button>
            </div>
          </div>
        )}

        {isConnected && (
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
