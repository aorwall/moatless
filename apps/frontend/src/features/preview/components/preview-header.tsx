import type { ServerCapabilities, ServerEntry } from '@moatless/api-client'
import { ExternalLink, MousePointer, RotateCcw } from 'lucide-react'
import { type KeyboardEvent, useCallback, useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/lib/components/ui/tooltip'
import { useFeatureFlag } from '@/lib/hooks/use-feature-flag'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { WorkspaceHeader } from '@/lib/layout/workspace-header'
import type { PreviewPublish } from '../types'
import { DeviceToggle } from './device-toggle'
import { InspectorUnavailableDialog } from './inspector-unavailable-dialog'
import { PublishButton } from './publish-button'
import { ViewTabs } from './view-tabs'

interface PreviewHeaderProps {
  path: string
  onNavigate: (path: string) => void
  isInspectorActive: boolean
  onToggleInspector: () => void
  onRequestAvailabilityCheck: () => void
  onReload: () => void
  isInspectorAvailable: boolean | null
  isInspectorDialogOpen: boolean
  onInspectorDialogOpenChange: (open: boolean) => void
  serverUrl?: string | undefined
  /** Dynamic server list — rendered as the workspace view tabs */
  servers?: ServerEntry[] | undefined
  /** Active workspace view — highlights the matching tab */
  view: string
  /** Switch the active workspace view */
  onViewChange: (view: string) => void
  /** Capabilities of the active server — controls which controls to show */
  capabilities?: ServerCapabilities | undefined
  /** Whether the sandbox is running (controls disabled when false) */
  isSandboxRunning?: boolean | undefined
  /** Publish/deploy state, injected by the composing feature (undefined = no publish). */
  publish?: PreviewPublish | undefined
}

export function PreviewHeader({
  path,
  onNavigate,
  isInspectorActive,
  onToggleInspector,
  onRequestAvailabilityCheck,
  onReload,
  isInspectorAvailable,
  isInspectorDialogOpen,
  onInspectorDialogOpenChange,
  serverUrl,
  servers,
  view,
  onViewChange,
  capabilities,
  isSandboxRunning = true,
  publish,
}: PreviewHeaderProps) {
  const [pathInput, setPathInput] = useState(path)
  const isNetlifyPublishEnabled = useFeatureFlag('NETLIFY_PUBLISH')
  const isMobile = useIsMobile()

  // Reset pathInput when path changes (using key prop pattern in parent is better)
  // But for internal component state, we need to track when path changes
  const [lastPath, setLastPath] = useState(path)

  if (path !== lastPath) {
    setPathInput(path)
    setLastPath(path)
  }

  const handlePathSubmit = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Enter') {
        let newPath = pathInput.trim()
        if (!newPath.startsWith('/')) {
          newPath = `/${newPath}`
        }

        // Disable inspector if active before navigating
        if (isInspectorActive) {
          onToggleInspector()
        }

        onNavigate(newPath)
      }
    },
    [pathInput, onNavigate, isInspectorActive, onToggleInspector],
  )

  const handleOpenInNewTab = useCallback(() => {
    if (serverUrl) {
      const fullUrl = `${serverUrl}${path}`
      window.open(fullUrl, '_blank', 'noopener,noreferrer')
    }
  }, [serverUrl, path])

  // Derive which controls to show from capabilities
  const showAddressBar = capabilities?.addressBar ?? true
  const showInspector = capabilities?.inspector ?? false
  const showReload = capabilities?.reload ?? true
  const showOpenExternal = capabilities?.openExternal ?? true
  const showDeviceToggle = capabilities?.viewport ?? false

  // Disable controls when sandbox is not running
  const controlsDisabled = !isSandboxRunning

  return (
    <WorkspaceHeader
      showToggle
      leading={<ViewTabs servers={servers ?? []} activeView={view} onViewChange={onViewChange} />}
      centerContent={
        <div className="flex items-center gap-1 md:gap-2 w-full justify-center">
          {/* Device Controls - Hidden on mobile */}
          {showDeviceToggle && !isMobile && <DeviceToggle />}

          {/* Reload button */}
          {showReload && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={onReload}
                  className="h-8 w-8 flex-shrink-0"
                  aria-label="Reload"
                  disabled={controlsDisabled}
                >
                  <RotateCcw className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Reload</TooltipContent>
            </Tooltip>
          )}

          {/* Address Bar - Flexible width on mobile */}
          {showAddressBar && (
            <Input
              type="text"
              value={pathInput}
              onChange={(e) => setPathInput(e.target.value)}
              onKeyDown={handlePathSubmit}
              placeholder="Enter path (e.g., /about)"
              className="flex-1 min-w-0 md:w-48 md:flex-initial h-8 text-sm"
              aria-label="Path"
              disabled={controlsDisabled}
            />
          )}

          {/* Tool Controls */}
          <div className="flex items-center gap-1 flex-shrink-0">
            {showInspector && (
              <>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant={isInspectorActive ? 'default' : 'ghost'}
                      size="icon"
                      onClick={() => {
                        // If availability unknown, request check first
                        if (isInspectorAvailable === null) {
                          onRequestAvailabilityCheck()
                          // The check result will be handled automatically in preview.tsx
                          // Inspector will auto-enable if available, or dialog will show if not
                        } else if (!isInspectorAvailable) {
                          onInspectorDialogOpenChange(true)
                        } else {
                          onToggleInspector()
                        }
                      }}
                      className="h-8 w-8"
                      aria-label="Toggle Inspector"
                      disabled={controlsDisabled}
                    >
                      <MousePointer className="h-4 w-4" />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>
                    {isInspectorAvailable === null
                      ? 'Check Inspector Availability'
                      : !isInspectorAvailable
                        ? 'Inspector Not Available'
                        : 'Toggle Element Inspector'}
                  </TooltipContent>
                </Tooltip>

                <InspectorUnavailableDialog
                  isOpen={isInspectorDialogOpen}
                  onOpenChange={onInspectorDialogOpenChange}
                />
              </>
            )}

            {showOpenExternal && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={handleOpenInNewTab}
                    className="h-8 w-8"
                    aria-label="Open in new tab"
                    disabled={controlsDisabled}
                  >
                    <ExternalLink className="h-4 w-4" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Open in New Tab</TooltipContent>
              </Tooltip>
            )}
          </div>
        </div>
      }
      rightContent={
        isNetlifyPublishEnabled && publish ? (
          <PublishButton
            canPublish={publish.canPublish && isSandboxRunning}
            siteUrl={publish.siteUrl ?? undefined}
            onPublish={publish.onPublish}
            isPending={publish.isPending}
          />
        ) : null
      }
    />
  )
}
