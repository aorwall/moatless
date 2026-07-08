import { Timer } from 'lucide-react'
import { SandboxActionButtons } from '@/features/sandbox/components/sandbox-action-buttons'
import { useSandboxStatus } from '@/features/tasks'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { useUpdateTaskTtl } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { cn } from '@/lib/utils'
import { TaskConnectionsSection } from './task-connections-section'
import { TaskInfoSection } from './task-header/task-info-section'

/** Idle stop: minutes of inactivity before the sandbox is stopped (0 = never). */
const IDLE_STOP_PRESETS: Array<{ label: string; minutes: number }> = [
  { label: '15m', minutes: 15 },
  { label: '1h', minutes: 60 },
  { label: '12h', minutes: 720 },
  { label: '24h', minutes: 1440 },
  { label: 'Never', minutes: 0 },
]

/** Human-readable status label + presentation for each sandbox lifecycle state. */
const SANDBOX_STATUS_META: Record<string, { label: string; description: string; dot: string }> = {
  not_created: {
    label: 'Not created',
    description: 'The sandbox has not been provisioned yet.',
    dot: 'bg-orange-500',
  },
  initializing: {
    label: 'Starting',
    description: 'The sandbox is being prepared…',
    dot: 'bg-amber-500 animate-pulse',
  },
  ready: {
    label: 'Running',
    description: 'The sandbox is up and ready.',
    dot: 'bg-green-500',
  },
  stopped: {
    label: 'Stopped',
    description: 'The sandbox was stopped.',
    dot: 'bg-muted-foreground/40',
  },
  removing: {
    label: 'Removing',
    description: 'The sandbox is being removed…',
    dot: 'bg-amber-500 animate-pulse',
  },
  removed: {
    label: 'Removed',
    description: 'The sandbox has been removed.',
    dot: 'bg-muted-foreground/40',
  },
  error: {
    label: 'Error',
    description: 'The sandbox encountered an error.',
    dot: 'bg-destructive',
  },
}

interface TaskControlPanelProps {
  className?: string
}

/**
 * Sandbox control view rendered in the right workspace pane (the "Sandbox" tab).
 * Leads with task details, then the current sandbox state with state-aware
 * lifecycle buttons and the idle-stop TTL, then connections.
 */
export function TaskControlPanel({ className }: TaskControlPanelProps) {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const { data: sandboxStatus } = useSandboxStatus(taskId, taskQuery.data)
  const updateTtl = useUpdateTaskTtl(taskId)
  const task = taskQuery.data

  if (!task) {
    return null
  }

  const environmentStatus = sandboxStatus?.environmentStatus
  const statusMeta = (environmentStatus && SANDBOX_STATUS_META[environmentStatus]) ?? {
    label: 'Unknown',
    description: 'Sandbox status is unavailable.',
    dot: 'bg-muted-foreground/40',
  }
  const environmentError = sandboxStatus?.environmentError

  return (
    <div className={cn('flex-1 min-h-0 overflow-y-auto', className)}>
      <div className="w-full px-4 md:px-6 pt-4 pb-6 space-y-4">
        <section className="rounded-md border">
          <div className="px-3 py-2 text-xs font-medium text-muted-foreground border-b">
            Details
          </div>
          <TaskInfoSection task={task} />
        </section>

        {/* Sandbox state + lifecycle controls */}
        <section className="rounded-md border">
          <div className="flex items-center justify-between gap-2 px-3 py-2 border-b">
            <div className="flex items-center gap-2">
              <span className={cn('inline-block h-2 w-2 rounded-full', statusMeta.dot)} />
              <span className="text-sm font-medium">Sandbox</span>
              <span className="text-xs text-muted-foreground">{statusMeta.label}</span>
            </div>
            <SandboxActionButtons taskId={taskId} environmentStatus={environmentStatus} />
          </div>
          <div className="space-y-3 px-3 py-3">
            <p className="text-sm text-muted-foreground">
              {environmentStatus === 'error' && environmentError
                ? environmentError
                : statusMeta.description}
            </p>
            <div>
              <div className="mb-1.5 flex items-center gap-1.5 text-xs text-muted-foreground">
                <Timer className="h-3 w-3" />
                Idle stop — stop the sandbox after inactivity
              </div>
              <div className="flex flex-wrap gap-2">
                {IDLE_STOP_PRESETS.map((preset) => {
                  const isActive = task.ttlMinutes === preset.minutes
                  return (
                    <Button
                      key={preset.minutes}
                      variant={isActive ? 'default' : 'outline'}
                      size="sm"
                      disabled={updateTtl.isPending}
                      onClick={() => {
                        if (!isActive) {
                          updateTtl.mutate({ ttlMinutes: preset.minutes })
                        }
                      }}
                    >
                      {preset.label}
                    </Button>
                  )
                })}
              </div>
            </div>
          </div>
        </section>

        <section className="rounded-md border">
          <div className="px-3 py-2 text-xs font-medium text-muted-foreground border-b">
            Connections
          </div>
          <TaskConnectionsSection taskId={taskId} />
        </section>
      </div>
    </div>
  )
}
