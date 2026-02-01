import { ExternalLink, MousePointer, RotateCcw } from 'lucide-react'
import { type KeyboardEvent, useCallback, useState } from 'react'
import { toast } from 'sonner'
import { useRepository } from '@/features/repositories/hooks/use-repositories'
import { WorkspaceHeader } from '@/features/tasks/components/workspace-header'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { useTaskView } from '@/features/tasks/hooks/use-task-view'
import { useDeployTask } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/lib/components/ui/tooltip'
import { useFeatureFlag } from '@/lib/hooks/use-feature-flag'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { getErrorMessage } from '@/lib/utils/error'
import type { ServerCapabilities } from '../api/servers'
import type { ServerEntry } from '../hooks/use-servers'
import { InspectorUnavailableDialog } from './inspector-unavailable-dialog'
import { PublishButton } from './publish-button'
import { ViewportToggle } from './viewport-toggle'

interface PreviewHeaderProps {
  serverManagement?: boolean
  path: string
  onNavigate: (path: string) => void
  isInspectorActive: boolean
  onToggleInspector: () => void
  onRequestAvailabilityCheck: () => void
  onReload: () => void
  isInspectorAvailable: boolean | null
  isInspectorDialogOpen: boolean
  onInspectorDialogOpenChange: (open: boolean) => void
  serverUrl?: string
  /** Dynamic server list — passed through to WorkspaceHeader for dynamic tabs */
  servers?: ServerEntry[]
  /** Capabilities of the active server — controls which controls to show */
  capabilities?: ServerCapabilities
}

export function PreviewHeader({
  serverManagement,
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
  capabilities,
}: PreviewHeaderProps) {
  const taskId = useTaskId()
  const { data: task } = useTaskDetailQuery(taskId)
  const { data: repository } = useRepository(task?.repositoryId ?? '')
  const vibe = task?.vibe
  const siteUrl = task?.siteUrl

  // Show publish button if vibe or has buildCommand
  const hasBuildCommand = Boolean(repository?.previewConfig?.buildCommand)
  const canPublish = vibe || hasBuildCommand

  const [pathInput, setPathInput] = useState(path)
  const isNetlifyPublishEnabled = useFeatureFlag('NETLIFY_PUBLISH')
  const isMobile = useIsMobile()

  // Get view state from URL
  const { view, setView } = useTaskView({ serverManagement })

  // Reset pathInput when path changes (using key prop pattern in parent is better)
  // But for internal component state, we need to track when path changes
  const [lastPath, setLastPath] = useState(path)

  if (path !== lastPath) {
    setPathInput(path)
    setLastPath(path)
  }

  // Deploy hook for publishing vibe tasks
  const deployTask = useDeployTask(taskId, {
    onSuccess: (data) => {
      if (data.success && data.siteUrl) {
        toast.success('Site published successfully!', {
          description: `Your site is live at ${data.siteUrl}`,
          action: {
            label: 'Open Site',
            onClick: () => window.open(data.siteUrl, '_blank', 'noopener,noreferrer'),
          },
        })
      } else if (data.error) {
        toast.error('Failed to publish site', {
          description: data.error,
        })
      }
    },
    onError: (error) => {
      toast.error('Failed to publish site', {
        description: getErrorMessage(error),
      })
    },
  })

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

  const handlePublish = useCallback(() => {
    deployTask.mutate()
  }, [deployTask])

  // Derive which controls to show from capabilities
  const showAddressBar = capabilities?.addressBar ?? true
  const showInspector = capabilities?.inspector ?? false
  const showReload = capabilities?.reload ?? true
  const showOpenExternal = capabilities?.openExternal ?? true
  const showViewport = capabilities?.viewport ?? false

  return (
    <WorkspaceHeader
      activeView={view}
      onViewChange={setView}
      showToggle={serverManagement}
      servers={servers}
      centerContent={
        <div className="flex items-center gap-1 md:gap-2 w-full">
          {/* Viewport Controls - Hidden on mobile */}
          {showViewport && !isMobile && <ViewportToggle />}

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
              className="flex-1 min-w-0 md:w-64 md:flex-initial h-8 text-sm"
              aria-label="Path"
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
        isNetlifyPublishEnabled ? (
          <PublishButton
            canPublish={canPublish}
            siteUrl={siteUrl}
            onPublish={handlePublish}
            isPending={deployTask.isPending}
          />
        ) : null
      }
    />
  )
}
