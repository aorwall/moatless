import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

interface ConfigurationSectionProps {
  children: ReactNode
  className?: string
}

export function ConfigurationSection({ children, className }: ConfigurationSectionProps) {
  return <div className={cn('space-y-6', className)}>{children}</div>
}
