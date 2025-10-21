import type { LucideIcon } from 'lucide-react'
import type * as React from 'react'
import { Button } from '@/lib/components/ui/button.tsx'
import { Popover, PopoverContent, PopoverTrigger } from '@/lib/components/ui/popover.tsx'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/lib/components/ui/tooltip.tsx'
import { cn } from '@/lib/utils.ts'

export interface PopoverSelectorProps {
  icon: LucideIcon
  value?: string
  displayValue?: string
  tooltip?: string
  disabled?: boolean
  loading?: boolean
  children: React.ReactNode
  className?: string
}

export function PopoverSelector({
  icon: Icon,
  value,
  displayValue,
  tooltip,
  disabled = false,
  loading = false,
  children,
  className,
}: PopoverSelectorProps) {
  const hasValue = value && value !== '' && value !== '__none__'
  const buttonText = hasValue ? displayValue || value : undefined

  return (
    <Popover>
      <Tooltip>
        <PopoverTrigger asChild>
          <TooltipTrigger asChild>
            <Button
              variant="outline"
              size="sm"
              disabled={disabled || loading}
              className={cn(
                'h-6 p-3 text-xs justify-start gap-1.5 rounded-lg',
                !hasValue && 'text-muted-foreground',
                className,
              )}
            >
              <Icon className="h-3.5 w-3.5 shrink-0" />
              {buttonText && (
                <span className="hidden sm:inline truncate max-w-[200px]">{buttonText}</span>
              )}
            </Button>
          </TooltipTrigger>
        </PopoverTrigger>
        <TooltipContent className="sm:hidden">
          <p>{tooltip || buttonText}</p>
        </TooltipContent>
      </Tooltip>
      <PopoverContent className="w-auto max-w-96 p-0" align="start">
        {children}
      </PopoverContent>
    </Popover>
  )
}
