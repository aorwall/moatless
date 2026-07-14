import type { BackgroundProcess, ScheduledJobInfo } from '@moatless/api-client'
import { Activity } from 'lucide-react'

interface BackgroundProcessesSectionProps {
  processes: BackgroundProcess[]
  scheduledJobs?: ScheduledJobInfo[] | undefined
}

/**
 * Lists the live `run_in_background` jobs (and ad-hoc daemons) still running in
 * the sandbox after the agent went idle. When a background-completion watch is
 * armed, notes that the agent will resume on its own once the work finishes.
 * Renders nothing when there is neither running work nor an armed watch.
 */
export function BackgroundProcessesSection({
  processes,
  scheduledJobs,
}: BackgroundProcessesSectionProps) {
  const watch = scheduledJobs?.find((job) => job.kind === 'backgroundWatch')

  if (processes.length === 0 && !watch) {
    return null
  }

  return (
    <section className="rounded-md border">
      <div className="flex items-center gap-2 px-3 py-2 border-b">
        <Activity className="h-3.5 w-3.5 text-muted-foreground" />
        <span className="text-sm font-medium">Background processes</span>
        {processes.length > 0 && (
          <span className="text-xs text-muted-foreground tabular-nums">{processes.length}</span>
        )}
      </div>
      <div className="space-y-2 px-3 py-3">
        {processes.length > 0 ? (
          <ul className="space-y-1.5">
            {processes.map((proc) => (
              <li key={proc.pid} className="flex items-start gap-2 text-xs">
                <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 font-mono text-muted-foreground tabular-nums">
                  {proc.pid}
                </span>
                <code className="min-w-0 break-all text-foreground/90">{proc.command}</code>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-xs text-muted-foreground">No background processes are running.</p>
        )}
        {watch?.kind === 'backgroundWatch' && (
          <p className="text-xs text-muted-foreground">
            The agent will resume automatically when {watch.processCount}{' '}
            {watch.processCount === 1 ? 'background job' : 'background jobs'} finish.
          </p>
        )}
      </div>
    </section>
  )
}
