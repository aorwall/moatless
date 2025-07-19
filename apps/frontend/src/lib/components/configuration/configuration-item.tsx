import type { ReactNode } from 'react'
import { Label } from '@/lib/components/ui/label'
import { cn } from '@/lib/utils'

interface ConfigurationItemProps {
  label: string
  description?: string
  htmlFor?: string
  children: ReactNode
  className?: string
}

export function ConfigurationItem({
  label,
  description,
  htmlFor,
  children,
  className,
}: ConfigurationItemProps) {
  // Generate ID for accessibility if not provided
  const inputId = htmlFor || `config-${label.toLowerCase().replace(/\s+/g, '-')}`

  return (
    <div className={cn('grid grid-cols-1 md:grid-cols-2 gap-4 items-start', className)}>
      {/* Left column: Label and description */}
      <div className="space-y-1">
        <Label htmlFor={inputId} className="text-sm font-medium text-foreground">
          {label}
        </Label>
        {description && <p className="text-sm text-muted-foreground">{description}</p>}
      </div>

      {/* Right column: Control */}
      <div>{children}</div>
    </div>
  )
}
