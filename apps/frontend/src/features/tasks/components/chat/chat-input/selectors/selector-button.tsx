import type { LucideIcon } from 'lucide-react'
import { Button } from '@/lib/components/ui/button'
import { cn } from '@/lib/utils'

export interface SelectorButtonProps {
  icon: LucideIcon
  label: string
  onClick?: (() => void) | undefined
  disabled?: boolean | undefined
  className?: string | undefined
}

export function SelectorButton({
  icon: Icon,
  label,
  onClick,
  disabled = false,
  className,
}: SelectorButtonProps) {
  return (
    <Button
      variant="outline"
      size="sm"
      onClick={onClick}
      disabled={disabled}
      className={cn('h-6 p-3 text-xs justify-start gap-1.5 rounded-lg', className)}
    >
      <Icon className="h-3.5 w-3.5 shrink-0" />
      <span className="hidden sm:inline truncate max-w-[200px]">{label}</span>
    </Button>
  )
}
