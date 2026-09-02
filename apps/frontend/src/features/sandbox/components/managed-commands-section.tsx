import type { ManagedCommand, ScheduledJobInfo } from '@moatless/api-client'
import { Activity } from 'lucide-react'

interface ManagedCommandsSectionProps {
  commands: ManagedCommand[]
  scheduledJobs?: ScheduledJobInfo[] | undefined
}

/**
 * Lists the long-running commands the agent registered through `moat cmd`,
 * running and recently finished. When a completion watch is armed, notes that
 * the agent will resume on its own once the work finishes. Renders nothing when
 * there is neither a command nor an armed watch.
 */
export function ManagedCommandsSection({ commands, scheduledJobs }: ManagedCommandsSectionProps) {
  const watch = scheduledJobs?.find((job) => job.kind === 'backgroundWatch')

  if (commands.length === 0 && !watch) {
    return null
  }

  const running = commands.filter((cmd) => cmd.state === 'running')

  return (
    <section className="rounded-md border">
      <div className="flex items-center gap-2 px-3 py-2 border-b">
        <Activity className="h-3.5 w-3.5 text-muted-foreground" />
        <span className="text-sm font-medium">Commands</span>
        {running.length > 0 && (
          <span className="text-xs text-muted-foreground tabular-nums">{running.length}</span>
        )}
      </div>
      <div className="space-y-2 px-3 py-3">
        {commands.length > 0 ? (
          <ul className="space-y-1.5">
            {commands.map((cmd) => (
              <li key={cmd.id} className="flex items-start gap-2 text-xs">
                <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 font-mono text-muted-foreground">
                  {outcomeLabel(cmd)}
                </span>
                <code className="min-w-0 break-all text-foreground/90">{cmd.label}</code>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-xs text-muted-foreground">No commands are running.</p>
        )}
        {watch?.kind === 'backgroundWatch' && (
          <p className="text-xs text-muted-foreground">
            The agent will resume automatically when {watch.commandCount}{' '}
            {watch.commandCount === 1 ? 'command finishes' : 'commands finish'}.
          </p>
        )}
      </div>
    </section>
  )
}

function outcomeLabel(cmd: ManagedCommand): string {
  switch (cmd.state) {
    case 'running':
      return `pid ${cmd.pid}`
    case 'exited':
      return cmd.exitCode === 0 ? 'ok' : `exit ${cmd.exitCode}`
    case 'timedOut':
      return `timeout ${cmd.timeoutSecs}s`
    case 'killed':
      return 'killed'
  }
}
