import { AlertCircle, CircleCheck, Loader2, Play } from 'lucide-react'
import { SandboxStatusOverlay } from '@/features/preview'
import { useSandboxRetry, useTaskSandboxStatusQuery } from '@/lib/api/task-sandbox-queries'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import { cn } from '@/lib/utils'
import { useFileViewState } from '../hooks/use-file-view-state'
import { useSelectedFile } from '../hooks/use-selected-file'
import { FileViewer } from './file-viewer'

interface FileOnlyViewerProps {
  taskId: string
}

type EnvironmentStatus =
  | 'not_created'
  | 'initializing'
  | 'ready'
  | 'stopped'
  | 'removing'
  | 'removed'
  | 'error'

function getStatusLabel(status: EnvironmentStatus | undefined, isLoading: boolean): string {
  if (isLoading) {
    return 'Checking sandbox'
  }

  switch (status) {
    case 'ready':
      return 'Sandbox ready'
    case 'initializing':
      return 'Starting sandbox'
    case 'stopped':
      return 'Sandbox stopped'
    case 'not_created':
      return 'Sandbox not running'
    case 'error':
      return 'Sandbox error'
    case 'removing':
      return 'Removing sandbox'
    case 'removed':
      return 'Sandbox removed'
    default:
      return 'Sandbox unavailable'
  }
}

function canStartSandbox(status: EnvironmentStatus | undefined): boolean {
  return (
    status === 'not_created' || status === 'stopped' || status === 'error' || status === 'removed'
  )
}

function FileOnlyTopBar({
  filePath,
  status,
  isLoading,
  isStarting,
  onStart,
}: {
  filePath: string | undefined
  status: EnvironmentStatus | undefined
  isLoading: boolean
  isStarting: boolean
  onStart: () => void
}) {
  const sandboxReady = status === 'ready'
  const showStartButton = canStartSandbox(status)
  const statusLabel = getStatusLabel(status, isLoading)

  return (
    <div className="h-11 border-b px-3 flex items-center gap-3 bg-background">
      <div className="min-w-0 flex-1 flex items-center gap-2">
        <span className="text-sm font-mono truncate">{filePath ?? 'No file selected'}</span>
      </div>
      <Badge
        variant={sandboxReady ? 'secondary' : 'outline'}
        className={cn(
          'h-6 gap-1.5',
          !sandboxReady && 'border-orange-300 text-orange-700 bg-orange-50',
          status === 'error' && 'border-destructive/40 text-destructive bg-destructive/10',
        )}
      >
        {isLoading || status === 'initializing' || isStarting ? (
          <Loader2 className="h-3 w-3 animate-spin" />
        ) : sandboxReady ? (
          <CircleCheck className="h-3 w-3" />
        ) : (
          <AlertCircle className="h-3 w-3" />
        )}
        {statusLabel}
      </Badge>
      {showStartButton && (
        <Button size="sm" variant="outline" onClick={onStart} disabled={isStarting}>
          {isStarting ? <Loader2 className="h-4 w-4 animate-spin" /> : <Play className="h-4 w-4" />}
          <span>Start Sandbox</span>
        </Button>
      )}
    </div>
  )
}

export function FileOnlyViewer({ taskId }: FileOnlyViewerProps) {
  const { selectedFile } = useSelectedFile()
  const { fileView, setFileView } = useFileViewState()
  const sandboxStatusQuery = useTaskSandboxStatusQuery(taskId)
  const sandboxStatus = sandboxStatusQuery.data
  const retryMutation = useSandboxRetry(taskId)

  const environmentStatus = sandboxStatus?.environmentStatus
  const displayStatus = environmentStatus ?? (sandboxStatusQuery.error ? 'error' : undefined)
  const environmentError = sandboxStatus?.environmentError ?? sandboxStatusQuery.error?.message
  const isSandboxReady = displayStatus === 'ready'

  return (
    <div className="h-full w-full bg-background flex flex-col">
      {!isSandboxReady && (
        <FileOnlyTopBar
          filePath={selectedFile}
          status={displayStatus}
          isLoading={sandboxStatusQuery.isLoading}
          isStarting={retryMutation.isPending}
          onStart={() => retryMutation.mutate()}
        />
      )}
      <div className="flex-1 min-h-0 relative">
        {isSandboxReady ? (
          <FileViewer
            taskId={taskId}
            filePath={selectedFile}
            viewMode={fileView}
            onViewModeChange={setFileView}
            hideHeaderActions
          />
        ) : displayStatus ? (
          <SandboxStatusOverlay
            environmentStatus={displayStatus}
            environmentError={environmentError}
            onRetry={() => retryMutation.mutate()}
            isRetrying={retryMutation.isPending}
            retryError={retryMutation.error}
          />
        ) : (
          <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
            Checking sandbox status...
          </div>
        )}
      </div>
    </div>
  )
}
