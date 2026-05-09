import { LayoutTemplate } from 'lucide-react'
import { useAgents } from '@/features/agents/hooks/use-agents'
import type { TaskTemplateResponse } from '@/features/task-templates/api/task-templates'
import { cn } from '@/lib/utils'

interface TemplateChipProps {
  template: TaskTemplateResponse
  onPick: (template: TaskTemplateResponse) => void
}

export function TemplateChip({ template, onPick }: TemplateChipProps) {
  const { data: agents } = useAgents()
  const agent = agents?.find((a) => a.id === template.agentType)
  const unavailable = !!agents && !!agent && !agent.installed

  const title = unavailable
    ? `${template.name} — connect ${agent?.label ?? template.agentType} to use this template`
    : (template.description ?? template.name)

  return (
    <button
      type="button"
      title={title}
      disabled={unavailable}
      onClick={() => onPick(template)}
      className={cn(
        'flex items-center gap-1.5 rounded-full border px-3 py-1.5 text-sm font-medium transition-colors',
        'border-border bg-background hover:bg-accent hover:text-accent-foreground',
        unavailable && 'cursor-not-allowed opacity-40',
      )}
    >
      <LayoutTemplate className="h-3.5 w-3.5 shrink-0" />
      <span className="max-w-[120px] truncate">{template.name}</span>
    </button>
  )
}
