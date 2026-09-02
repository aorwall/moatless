import type { ScheduledJobInfo } from '@moatless/api-client'
import { Activity, Clock, Timer } from 'lucide-react'
import { formatRelativeTime } from '@/features/tasks/utils/format-relative-time'
import { Button } from '@/lib/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover'

type Props = { jobs: ScheduledJobInfo[] }

export function ScheduledJobsPill({ jobs }: Props) {
  if (jobs.length === 0) {
    return null
  }
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="ghost" size="sm" className="h-7 gap-1 px-2" title="Scheduled jobs">
          <Clock className="h-3.5 w-3.5" />
          <span className="text-xs tabular-nums">{jobs.length}</span>
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-72 p-1" align="end">
        <div className="flex flex-col">
          {jobs.map((job) => (
            <ScheduledJobRow key={job.id} job={job} />
          ))}
        </div>
      </PopoverContent>
    </Popover>
  )
}

function ScheduledJobRow({ job }: { job: ScheduledJobInfo }) {
  if (job.kind === 'backgroundWatch') {
    const count = job.commandCount
    return (
      <div className="flex gap-2 px-2 py-1.5 rounded-sm hover:bg-accent">
        <Activity className="h-3.5 w-3.5 mt-0.5 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <div className="text-xs font-medium truncate">
            Waiting for {count} {count === 1 ? 'command' : 'commands'}
          </div>
          <div className="text-[11px] text-muted-foreground truncate">
            {job.commands.length > 0 ? job.commands.join('; ') : 'Resumes when finished'}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="flex gap-2 px-2 py-1.5 rounded-sm hover:bg-accent">
      <Timer className="h-3.5 w-3.5 mt-0.5 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="text-xs font-medium truncate">{job.reason}</div>
        <div className="text-[11px] text-muted-foreground tabular-nums">
          {formatRelativeTime(job.wakeupAtUnixMs)}
        </div>
      </div>
    </div>
  )
}
