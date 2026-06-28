import { useNavigate } from '@tanstack/react-router'
import { Check, Globe, Lock, Power, RefreshCw, Timer, XCircle } from 'lucide-react'
import { toast } from 'sonner'
import { useSandboxRestart, useSandboxStatus, useSandboxStop } from '@/features/tasks'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import {
  useCloseTask,
  useUpdateTaskTtl,
  useUpdateTaskVisibility,
} from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { cn } from '@/lib/utils'
import { TaskConnectionsSection } from './task-connections-section'
import { TaskInfoSection } from './task-header/task-info-section'

const TTL_PRESETS: Array<{ label: string; minutes: number }> = [
  { label: '15 minutes', minutes: 15 },
  { label: '1 hour', minutes: 60 },
  { label: '12 hours', minutes: 720 },
  { label: '24 hours', minutes: 1440 },
  { label: 'Indefinite', minutes: 0 },
]

interface TaskControlPanelProps {
  className?: string
}

/**
 * Full-height controls panel rendered in place of the chat view. Shows task
 * details (reusing TaskInfoSection) plus sandbox/task actions that previously
 * lived in the header's actions popover.
 */
export function TaskControlPanel({ className }: TaskControlPanelProps) {
  const taskId = useTaskId()
  const taskQuery = useTaskDetailQuery(taskId)
  const { data: sandboxStatus } = useSandboxStatus(taskId, taskQuery.data)
  const updateVisibility = useUpdateTaskVisibility(taskId)
  const sandboxRestart = useSandboxRestart(taskId)
  const sandboxStop = useSandboxStop(taskId)
  const updateTtl = useUpdateTaskTtl(taskId)
  const navigate = useNavigate()
  const closeTask = useCloseTask(taskId, {
    onSuccess: () => navigate({ to: '/' }),
    onError: (err) => toast.error('Failed to close task', { description: err.message }),
  })
  const task = taskQuery.data

  if (!task) {
    return null
  }

  const { visibility } = task

  return (
    <div className={cn('flex-1 min-h-0 overflow-y-auto', className)}>
      <div className="max-w-md mx-auto p-4 space-y-4">
        <section className="rounded-md border">
          <div className="px-3 py-2 text-xs font-medium text-muted-foreground border-b">
            Details
          </div>
          <TaskInfoSection task={task} sandbox={sandboxStatus} />
        </section>

        <section className="rounded-md border">
          <div className="px-3 py-2 text-xs font-medium text-muted-foreground border-b">
            Connections
          </div>
          <TaskConnectionsSection taskId={taskId} />
        </section>

        <section className="rounded-md border p-1">
          <Button
            variant="ghost"
            className="w-full justify-start gap-2 h-9 px-2"
            disabled={updateVisibility.isPending}
            onClick={() => updateVisibility.mutate(visibility === 'public' ? 'private' : 'public')}
          >
            {visibility === 'public' ? (
              <>
                <Lock className="h-4 w-4" />
                Make Private
              </>
            ) : (
              <>
                <Globe className="h-4 w-4" />
                Make Public
              </>
            )}
          </Button>
          <Button
            variant="ghost"
            className="w-full justify-start gap-2 h-9 px-2"
            disabled={sandboxRestart.isPending}
            onClick={() => sandboxRestart.mutate()}
          >
            <RefreshCw className={`h-4 w-4 ${sandboxRestart.isPending ? 'animate-spin' : ''}`} />
            Restart Sandbox
          </Button>
          <Button
            variant="ghost"
            className="w-full justify-start gap-2 h-9 px-2"
            disabled={sandboxStop.isPending}
            onClick={() => sandboxStop.mutate()}
          >
            <Power className="h-4 w-4" />
            Stop Sandbox
          </Button>
        </section>

        <section className="rounded-md border p-1">
          <div className="flex items-center gap-2 px-2 pt-1 pb-0.5 text-xs text-muted-foreground">
            <Timer className="h-3 w-3" />
            Sandbox TTL
          </div>
          {TTL_PRESETS.map((preset) => {
            const isActive = task.ttlMinutes === preset.minutes
            return (
              <Button
                key={preset.minutes}
                variant="ghost"
                className="w-full justify-start gap-2 h-9 px-2"
                disabled={updateTtl.isPending}
                onClick={() => {
                  if (!isActive) {
                    updateTtl.mutate(preset.minutes)
                  }
                }}
              >
                <Check className={`h-4 w-4 ${isActive ? 'opacity-100' : 'opacity-0'}`} />
                {preset.label}
              </Button>
            )
          })}
        </section>

        <section className="rounded-md border p-1">
          <Button
            variant="ghost"
            className="w-full justify-start gap-2 h-9 px-2 text-destructive hover:text-destructive"
            disabled={closeTask.isPending}
            onClick={() => closeTask.mutate()}
          >
            <XCircle className="h-4 w-4" />
            Close Task
          </Button>
        </section>
      </div>
    </div>
  )
}
