import type { ComponentType, SVGProps } from 'react'

interface DetailEmptyStateProps {
  icon: ComponentType<SVGProps<SVGSVGElement>>
  title: string
  description: string
}

/**
 * Centered placeholder for a master–detail pane when nothing is selected
 * (the index route of an admin section).
 */
export function DetailEmptyState({ icon: Icon, title, description }: DetailEmptyStateProps) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center text-muted-foreground">
      <Icon className="h-10 w-10 opacity-50" />
      <p className="font-medium">{title}</p>
      <p className="text-sm">{description}</p>
    </div>
  )
}
