import type { TaskTemplateResponse } from '@/features/task-templates/api/task-templates'
import { TemplateChip } from '@/features/task-templates/components/template-chip'
import { useFeaturedTaskTemplates } from '@/features/task-templates/hooks/use-task-templates'

interface TemplateChipsRowProps {
  onPick: (template: TaskTemplateResponse) => void
}

export function TemplateChipsRow({ onPick }: TemplateChipsRowProps) {
  const { data: templates } = useFeaturedTaskTemplates({ limit: 6 })

  if (!templates || templates.length === 0) {
    return null
  }

  return (
    <div className="flex flex-wrap gap-2 mb-3">
      {templates.map((template) => (
        <TemplateChip key={template.id} template={template} onPick={onPick} />
      ))}
    </div>
  )
}
