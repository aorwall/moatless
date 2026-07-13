import { Server } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useSandboxRetry } from '@/lib/api/task-sandbox-queries'
import { StackedConsoleSplit } from '@/lib/layout/stacked-console-split'
import { cn } from '@/lib/utils'
import { useConsoleLogs } from '../hooks/use-console-logs'
import { usePreviewInspector } from '../hooks/use-preview-inspector'
import { usePreviewMessages } from '../hooks/use-preview-messages'
import { usePreviewNavigation } from '../hooks/use-preview-navigation'
import { usePreviewDevice, usePreviewPath } from '../hooks/use-preview-view'
import { useScriptHealth } from '../hooks/use-script-health'
import type { PreviewProps } from '../types'
import { DeviceFrame } from './device-frame'
import { InspectorOverlay } from './inspector-overlay'
import { LogsPanel } from './logs-panel'
import { PreviewHeader } from './preview-header'
import { ProxyErrorOverlay } from './proxy-error-overlay'
import { ResourceUsageFooter } from './resource-usage-footer'
import { SandboxStatusOverlay } from './sandbox-status-overlay'
import { ServerStatusView } from './server-status-view'

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
  // null = follow the auto-expand default; true/false = user's explicit choice.
  const [consoleExpandedOverride, setConsoleExpandedOverride] = useState<boolean | null>(null)
  const [isLoaded, setIsLoaded] = useState(false)
  const [hasProxyError, setHasProxyError] = useState(false)

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

  const { consoleLogs, appendConsoleMessage, clearConsoleLogs } = useConsoleLogs()

  const { markScriptLoaded, recordHeartbeat, markNotInjected, resetScriptHealth } =
    useScriptHealth()

  const {
    isInspectorActive,
    isInspectorAvailable,
    isInspectorDialogOpen,
    setIsInspectorDialogOpen,
    requestAvailabilityCheck,
    handleInspectorAvailability,
    toggleInspector,
    handleElementSelected,
    deactivateInspector,
    resetInspector,
  } = usePreviewInspector({ iframeRef, onElementSelected, onToggleElementContext })

  const { iframeTargetPath, setIframeTargetPath, iframeSrcKey, navigatePreview, reloadIframe } =
    usePreviewNavigation({ iframeRef, path, setPath, serverStatus: serverStatus?.status })

  // Reset state when active server changes
  // biome-ignore lint/correctness/useExhaustiveDependencies: activeServer is the trigger for resetting state
  useEffect(() => {
    setIsLoaded(false)
    resetInspector()
    resetScriptHealth()
    clearConsoleLogs()
    setHasProxyError(false)
    setPath('/')
    setIframeTargetPath('/')
    setDevice('desktop')
    // Re-apply the auto-expand default for the newly selected server.
    setConsoleExpandedOverride(null)
  }, [activeServer])

  // Cleanup when server stops
  useEffect(() => {
    if (serverStatus?.status === 'stopped') {
      clearConsoleLogs()
      resetInspector()
      setIsLoaded(false)
      resetScriptHealth()
      // Keep path as-is so server can resume at same path when restarted
    }
  }, [serverStatus?.status, clearConsoleLogs, resetInspector, resetScriptHealth])

  // Clear proxy error when server becomes ready/started
  useEffect(() => {
    if (serverStatus?.status === 'started') {
      setHasProxyError(false)
    }
  }, [serverStatus?.status])

  // Helper: Handle route change message
  const handleRouteChange = useCallback(
    (payload: { path: string }) => {
      setPath(payload.path)
    },
    [setPath],
  )

  const handleProxyError = useCallback(() => {
    setHasProxyError(true)
  }, [])

  usePreviewMessages({
    onConsoleMessage: appendConsoleMessage,
    onElementClicked: onElementSelected,
    onRouteChange: handleRouteChange,
    onInspectorScriptLoaded: markScriptLoaded,
    onHeartbeat: recordHeartbeat,
    onInspectorAvailability: handleInspectorAvailability,
    onTroubleshootRequest,
    onProxyError: handleProxyError,
    onChatToggleRequest,
  })

  // URL is automatically updated via optimistic updates in GlobalEventStreamProvider
  // Use iframeTargetPath (not path) to avoid reloading iframe on every navigation
  const baseUrl = serverStatus?.url
  const previewUrl = baseUrl ? `${baseUrl}${iframeTargetPath}` : null

  // Expose navigation function to parent component
  useEffect(() => {
    if (onNavigationReady) {
      onNavigationReady(navigatePreview)
    }
  }, [onNavigationReady, navigatePreview])

  const reloadPreview = useCallback(() => {
    // Disable inspector on reload
    deactivateInspector()
    reloadIframe()
  }, [deactivateInspector, reloadIframe])

  const handleIframeLoad = useCallback(() => {
    setHasProxyError(false)
    setIsLoaded(true)

    // Reset script health status when iframe loads
    // If script is injected, we'll receive INSPECTOR_SCRIPT_LOADED message
    markNotInjected()

    // Don't reset isInspectorAvailable - it will be updated when the script
    // sends the INSPECTOR_AVAILABILITY message. Resetting here causes race conditions.

    // Script injection is now handled server-side to avoid cross-origin issues
    // The preview environment service automatically injects the inspector script
    // into the HTML files before serving them
  }, [markNotInjected])

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
          resources && isSandboxRunning ? <ResourceUsageFooter resources={resources} /> : null
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
                onElementSelected={handleElementSelected}
              />
            )}
          </DeviceFrame>
        )}
      </StackedConsoleSplit>
    </div>
  )
}
