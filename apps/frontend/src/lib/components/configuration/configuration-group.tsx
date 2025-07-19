import type { ReactNode } from 'react'
import { Separator } from '@/lib/components/ui/separator'
import { cn } from '@/lib/utils'

interface ConfigurationGroupProps {
  title?: string
  children: ReactNode
  showSeparator?: boolean
  className?: string
}

export function ConfigurationGroup({
  title,
  children,
  showSeparator = true,
  className,
}: ConfigurationGroupProps) {
  return (
    <>
      <div className={cn('space-y-4', className)}>
        {title && <h3 className="text-base font-semibold text-foreground">{title}</h3>}
        {children}
      </div>
      {showSeparator && <Separator className="my-6" />}
    </>
  )
}
