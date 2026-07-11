import { Play, RefreshCw, RotateCcw, Square, Trash2 } from 'lucide-react'
import {
  useSandboxCleanup,
  useSandboxRedeploy,
  useSandboxRestart,
  useSandboxRetry,
  useSandboxStop,
} from '@/features/tasks/hooks/use-sandbox-status'
import { Button } from '@/lib/components/ui/button'

interface SandboxActionButtonsProps {
  taskId: string
  environmentStatus?: string | undefined
  compact?: boolean | undefined
}

export function SandboxActionButtons({
  taskId,
  environmentStatus,
  compact = false,
}: SandboxActionButtonsProps) {
  const provision = useSandboxRetry(taskId)
  const restart = useSandboxRestart(taskId)
  const stop = useSandboxStop(taskId)
  const redeploy = useSandboxRedeploy(taskId)
  const cleanup = useSandboxCleanup(taskId)

  const isAnyLoading =
    provision.isPending ||
    restart.isPending ||
    stop.isPending ||
    redeploy.isPending ||
    cleanup.isPending

  const size = compact ? 'icon' : 'sm'
  const iconSize = compact ? 'h-3.5 w-3.5' : 'h-4 w-4'

  return (
    <div className="flex items-center gap-1">
      {(environmentStatus === 'not_created' || environmentStatus === 'stopped') && (
        <Button
          variant="outline"
          size={size}
          onClick={() => provision.mutate()}
          disabled={isAnyLoading}
          title="Start sandbox"
        >
          <Play className={iconSize} />
          {!compact && <span className="ml-1">Start</span>}
        </Button>
      )}

      {environmentStatus === 'ready' && (
        <Button
          variant="outline"
          size={size}
          onClick={() => restart.mutate()}
          disabled={isAnyLoading}
          title="Restart sandbox"
        >
          <RefreshCw className={iconSize} />
          {!compact && <span className="ml-1">Restart</span>}
        </Button>
      )}

      {(environmentStatus === 'ready' || environmentStatus === 'initializing') && (
        <Button
          variant="outline"
          size={size}
          onClick={() => stop.mutate()}
          disabled={isAnyLoading}
          title="Stop sandbox"
        >
          <Square className={iconSize} />
          {!compact && <span className="ml-1">Stop</span>}
        </Button>
      )}

      {environmentStatus === 'error' && (
        <>
          <Button
            variant="outline"
            size={size}
            onClick={() => redeploy.mutate()}
            disabled={isAnyLoading}
            title="Redeploy sandbox"
          >
            <RotateCcw className={iconSize} />
            {!compact && <span className="ml-1">Redeploy</span>}
          </Button>
          <Button
            variant="outline"
            size={size}
            onClick={() => cleanup.mutate()}
            disabled={isAnyLoading}
            title="Cleanup sandbox"
          >
            <Trash2 className={iconSize} />
            {!compact && <span className="ml-1">Cleanup</span>}
          </Button>
        </>
      )}
    </div>
  )
}
