import { ExternalLink, MousePointer, RotateCcw } from 'lucide-react'
import { type KeyboardEvent, useCallback, useState } from 'react'
import { toast } from 'sonner'
import { useServerStatus } from '@/features/preview'
import { WorkspaceHeader } from '@/features/tasks/components/workspace-header'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { useDeployTask } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { Input } from '@/lib/components/ui/input'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/lib/components/ui/tooltip'
import { useFeatureFlag } from '@/lib/hooks/use-feature-flag'
import { getErrorMessage } from '@/lib/utils/error'
import type { ScriptHealthStatus } from '../types'
import { InspectorUnavailableDialog } from './inspector-unavailable-dialog'
import { PublishButton } from './publish-button'
import { ServerControlPopover } from './server-control-popover'
import { ViewportToggle } from './viewport-toggle'

interface PreviewHeaderProps {
  serverManagement?: boolean
  path: string
  onNavigate: (path: string) => void
  isInspectorActive: boolean
  onToggleInspector: () => void
  onRequestAvailabilityCheck: () => void
  scriptHealthStatus: ScriptHealthStatus
  lastHeartbeat: number | null
  onReload: () => void
  isInspectorAvailable: boolean | null
  isInspectorDialogOpen: boolean
  onInspectorDialogOpenChange: (open: boolean) => void
}

export function PreviewHeader({
  serverManagement,
  path,
  onNavigate,
  isInspectorActive,
  onToggleInspector,
  onRequestAvailabilityCheck,
  scriptHealthStatus,
  onReload,
  isInspectorAvailable,
  isInspectorDialogOpen,
  onInspectorDialogOpenChange,
}: PreviewHeaderProps) {
  const taskId = useTaskId()
  const { data: task } = useTaskDetailQuery(taskId)
  const vibe = task?.vibe
  const iterations = task?.iterations ?? []

  // Get the latest iteration to check for site URL
  const latestIteration = iterations?.[iterations.length - 1]
  const siteUrl = latestIteration?.siteUrl

  const [pathInput, setPathInput] = useState(path)
  const [isServerPopoverOpen, setIsServerPopoverOpen] = useState(false)
  const isNetlifyPublishEnabled = useFeatureFlag('NETLIFY_PUBLISH')

  // Reset pathInput when path changes (using key prop pattern in parent is better)
  // But for internal component state, we need to track when path changes
  const [lastPath, setLastPath] = useState(path)

  if (path !== lastPath) {
    setPathInput(path)
    setLastPath(path)
  }

  // Server management hooks
  const { data: serverStatus } = useServerStatus(taskId)

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
    if (serverStatus?.url) {
      const fullUrl = `${serverStatus.url}${path}`
      window.open(fullUrl, '_blank', 'noopener,noreferrer')
    }
  }, [serverStatus?.url, path])

  const handlePublish = useCallback(() => {
    deployTask.mutate()
  }, [deployTask])

  return (
    <WorkspaceHeader
      showToggle={serverManagement}
      centerContent={
        <div className="flex items-center gap-2">
          {/* Viewport Controls */}
          <ViewportToggle />

          {/* Navigation Controls */}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                onClick={onReload}
                className="h-8 w-8"
                aria-label="Reload"
              >
                <RotateCcw className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Reload Preview</TooltipContent>
          </Tooltip>

          {/* Address Bar */}
          <Input
            type="text"
            value={pathInput}
            onChange={(e) => setPathInput(e.target.value)}
            onKeyDown={handlePathSubmit}
            placeholder="Enter path (e.g., /about)"
            className="w-64 h-8 text-sm"
            aria-label="Path"
          />

          {/* Tool Controls */}
          <div className="flex items-center gap-1">
            <ServerControlPopover
              taskId={taskId}
              serverStatus={serverStatus}
              scriptHealthStatus={scriptHealthStatus}
              isOpen={isServerPopoverOpen}
              onOpenChange={setIsServerPopoverOpen}
            />

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
          </div>
        </div>
      }
      rightContent={
        isNetlifyPublishEnabled ? (
          <PublishButton
            vibe={vibe}
            siteUrl={siteUrl}
            onPublish={handlePublish}
            isPending={deployTask.isPending}
          />
        ) : null
      }
    />
  )
}
