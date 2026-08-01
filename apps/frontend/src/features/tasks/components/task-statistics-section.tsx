import { Activity, Bot, Wrench } from 'lucide-react'
import { MessageTokenUsageChart } from '@/features/tasks/components/message-token-usage-chart'
import { useTaskStatisticsQuery } from '@/features/tasks/hooks/use-task-queries'
import { Badge } from '@/lib/components/ui/badge'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { cn } from '@/lib/utils'

interface TaskStatisticsSectionProps {
  taskId: string
}

const numberFormat = new Intl.NumberFormat()

export function TaskStatisticsSection({ taskId }: TaskStatisticsSectionProps) {
  const { data: stats, isLoading } = useTaskStatisticsQuery(taskId)

  if (isLoading) {
    return (
      <div className="space-y-3 px-3 py-3">
        <Skeleton className="h-12 w-full" />
        <Skeleton className="h-20 w-full" />
        <Skeleton className="h-16 w-full" />
      </div>
    )
  }

  if (!stats) {
    return <div className="px-3 py-3 text-sm text-muted-foreground">Statistics unavailable</div>
  }

  const hasMessageUsage = stats.messageTokenUsage.length > 0
  const hasToolCalls = stats.toolCalls.length > 0
  const hasSkills = stats.skills.length > 0
  // A task that ran before tool calls were stored as their own rows has no
  // record of them. Saying "no tool calls" there would be a wrong answer
  // rather than a missing one, so say which it is.
  const toolCallsRecorded = stats.toolCallsAvailable !== false

  return (
    <div className="space-y-4 px-3 py-3">
      <div className="grid grid-cols-2 gap-2 text-sm">
        <Metric label="Input" value={stats.tokenTotals.inputTokens} />
        <Metric label="Output" value={stats.tokenTotals.outputTokens} />
        <Metric label="Cached input" value={stats.tokenTotals.cachedInputTokens} />
        <Metric label="Total" value={stats.tokenTotals.totalTokens} strong />
      </div>

      <div className="grid grid-cols-2 gap-2 text-xs">
        <MetaItem icon={Bot} label="Agent" value={stats.agentType} />
        <MetaItem icon={Activity} label="Model" value={stats.model ?? 'Not recorded'} />
      </div>

      <div>
        <div className="mb-2 text-xs font-medium text-muted-foreground">Token usage by message</div>
        {hasMessageUsage ? (
          <MessageTokenUsageChart messages={stats.messageTokenUsage} />
        ) : (
          <p className="text-sm text-muted-foreground">No token usage recorded</p>
        )}
      </div>

      <div>
        <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <Wrench className="h-3 w-3" />
          Tool calls
        </div>
        {hasToolCalls ? (
          <div className="space-y-1.5">
            {stats.toolCalls.map((tool) => {
              const failureRate = tool.totalCalls > 0 ? tool.failedCalls / tool.totalCalls : 0
              return (
                <div
                  key={tool.toolName}
                  className="flex items-center justify-between gap-3 text-sm"
                >
                  <span className="truncate font-mono text-xs">{tool.toolName}</span>
                  <div className="flex shrink-0 items-center gap-2 text-xs tabular-nums">
                    <span>{numberFormat.format(tool.totalCalls)}</span>
                    <Badge
                      variant="secondary"
                      className={cn(
                        'h-5 px-1.5 font-normal',
                        tool.failedCalls > 0 && 'bg-destructive/10 text-destructive',
                      )}
                    >
                      {numberFormat.format(tool.failedCalls)} failed
                      {tool.totalCalls > 0 ? ` (${Math.round(failureRate * 100)}%)` : ''}
                    </Badge>
                  </div>
                </div>
              )
            })}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            {toolCallsRecorded ? 'No tool calls' : 'This task ran before tool calls were recorded'}
          </p>
        )}
      </div>

      <div>
        <div className="mb-2 text-xs font-medium text-muted-foreground">Skills used</div>
        {hasSkills ? (
          <div className="flex flex-wrap gap-1.5">
            {stats.skills.map((skill) => (
              <Badge key={skill.skillName} variant="outline" className="font-normal">
                {skill.skillName}
                <span className="ml-1 tabular-nums">{numberFormat.format(skill.totalCalls)}</span>
              </Badge>
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">No skill usage recorded</p>
        )}
      </div>
    </div>
  )
}

function Metric({
  label,
  value,
  strong = false,
}: {
  label: string
  value: number
  strong?: boolean
}) {
  return (
    <div className="rounded border px-2 py-1.5">
      <div className="text-[11px] text-muted-foreground">{label}</div>
      <div className={cn('tabular-nums', strong ? 'font-semibold' : 'font-medium')}>
        {numberFormat.format(value)}
      </div>
    </div>
  )
}

function MetaItem({
  icon: Icon,
  label,
  value,
}: {
  icon: typeof Bot
  label: string
  value: string
}) {
  return (
    <div className="flex min-w-0 items-center gap-2 rounded border px-2 py-1.5">
      <Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <div className="min-w-0">
        <div className="text-[11px] text-muted-foreground">{label}</div>
        <div className="truncate font-medium" title={value}>
          {value}
        </div>
      </div>
    </div>
  )
}
