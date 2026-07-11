import type React from 'react'
import { cn } from '@/lib/utils'

interface ChatScrollAreaProps {
  children: React.ReactNode
  className?: string | undefined
}

export function ChatScrollArea({ children, className }: ChatScrollAreaProps) {
  return <div className={cn('h-full flex flex-col relative', className)}>{children}</div>
}
