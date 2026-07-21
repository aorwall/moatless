import type { SandboxStatusFullResponse } from '@moatless/api-client'
import { ArrowUpCircle, CheckCircle2, ExternalLink, Pencil, Timer } from 'lucide-react'
import { useState } from 'react'
import { BackgroundProcessesSection } from '@/features/sandbox/components/background-processes-section'
import { SandboxActionButtons } from '@/features/sandbox/components/sandbox-action-buttons'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useSandboxStatus } from '@/features/tasks/hooks/use-sandbox-status'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { useUpdateTaskMetadata, useUpdateTaskTtl } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { cn } from '@/lib/utils'
import { TaskConnectionsSection } from './task-connections-section'
import { EditTaskDialog } from './task-header/edit-task-dialog'
import { TaskInfoSection } from './task-header/task-info-section'
import { TaskRelationsSection } from './task-relations-section'
import { TaskStatisticsSection } from './task-statistics-section'

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

/**
 * Sandbox desired state + docker image freshness. Shows the image the sandbox
 * is currently running and, when a newer image would be used on redeploy, an
 * "Update available" hint with the target image.
 */
function SandboxImageInfo({ status }: { status: SandboxStatusFullResponse | undefined }) {
  const desiredState = status?.sandboxDesiredState
  const currentImage = status?.currentSandboxImage
  const latestImage = status?.latestSandboxImage
  const imageUpToDate = status?.imageUpToDate

  if (!(desiredState || currentImage || latestImage)) {
    return null
  }

  return (
    <div className="space-y-1.5 rounded-md bg-muted/40 p-2 text-xs">
      {desiredState && (
        <div className="flex items-center justify-between gap-2">
          <span className="text-muted-foreground">Desired state</span>
          <span className="font-medium capitalize">{desiredState}</span>
        </div>
      )}
      <div className="flex items-start justify-between gap-2">
        <span className="shrink-0 text-muted-foreground">Image</span>
        <span
          className="break-all text-right font-mono text-[11px]"
          title={currentImage ?? undefined}
        >
          {currentImage ?? 'Not deployed'}
        </span>
      </div>
      {imageUpToDate === false && (
        <div className="flex items-start justify-between gap-2">
          <span className="inline-flex shrink-0 items-center gap-1 text-amber-600 dark:text-amber-500">
            <ArrowUpCircle className="h-3 w-3" />
            Update available
          </span>
          <span
            className="break-all text-right font-mono text-[11px] text-muted-foreground"
            title={latestImage ?? undefined}
          >
            {latestImage}
          </span>
        </div>
      )}
      {imageUpToDate === true && (
        <div className="inline-flex items-center gap-1 text-green-600 dark:text-green-500">
          <CheckCircle2 className="h-3 w-3" />
          On the latest image
        </div>
      )}
    </div>
  )
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
  const [isEditDialogOpen, setIsEditDialogOpen] = useState(false)
  const updateMetadata = useUpdateTaskMetadata(taskId, {
    onSuccess: () => setIsEditDialogOpen(false),
  })
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

  // Preview server to link out to: prefer the one literally named "preview",
  // then the default server, then the first. Only linkable once the backend has
  // assigned a `url` (server is running) and it may be opened externally.
  const servers = sandboxStatus?.servers
  const previewServer =
    servers?.find((s) => s.name === 'preview') ?? servers?.find((s) => s.default) ?? servers?.[0]
  const previewUrl =
    previewServer?.url && previewServer.capabilities.openExternal ? previewServer.url : undefined

  return (
    <div className={cn('flex-1 min-h-0 overflow-y-auto', className)}>
      <div className="w-full px-4 md:px-6 pt-4 pb-6 space-y-4">
        <section className="rounded-md border">
          <div className="flex items-center justify-between gap-2 px-3 py-2 border-b">
            <span className="text-xs font-medium text-muted-foreground">Details</span>
            <Button
              variant="ghost"
              size="sm"
              className="h-6 gap-1 px-2 text-xs"
              onClick={() => setIsEditDialogOpen(true)}
            >
              <Pencil className="h-3 w-3" />
              Edit
            </Button>
          </div>
          <TaskInfoSection task={task} />
        </section>

        <TaskRelationsSection task={task} />

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
            <SandboxImageInfo status={sandboxStatus} />
            {previewUrl && (
              <a
                href={previewUrl}
                target="_blank"
                rel="noopener noreferrer"
                title={previewUrl}
                className="flex items-center gap-2 text-sm text-primary hover:underline"
              >
                <ExternalLink className="h-3.5 w-3.5 shrink-0" />
                <span className="truncate">Open {previewServer?.label ?? 'preview'}</span>
              </a>
            )}
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

        <BackgroundProcessesSection
          processes={sandboxStatus?.backgroundProcesses ?? []}
          scheduledJobs={sandboxStatus?.scheduledJobs}
        />

        <section className="rounded-md border">
          <div className="px-3 py-2 text-xs font-medium text-muted-foreground border-b">
            Connections
          </div>
          <TaskConnectionsSection taskId={taskId} />
        </section>

        <section className="rounded-md border">
          <div className="px-3 py-2 text-xs font-medium text-muted-foreground border-b">
            Statistics
          </div>
          <TaskStatisticsSection taskId={taskId} />
        </section>
      </div>

      <EditTaskDialog
        key={`${isEditDialogOpen ? 'open' : 'closed'}:${task.name ?? ''}:${task.description ?? ''}`}
        isOpen={isEditDialogOpen}
        onOpenChange={setIsEditDialogOpen}
        currentName={task.name || ''}
        currentDescription={task.description || ''}
        onSubmit={(updates) => updateMetadata.mutate(updates)}
        isPending={updateMetadata.isPending}
      />
    </div>
  )
}
