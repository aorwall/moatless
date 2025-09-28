import { cn } from '@/lib/utils'

interface ChatViewProps {
  children: React.ReactNode
  className?: string
}

export function ChatView({ children, className }: ChatViewProps) {
  return <div className={cn('h-full flex flex-col', className)}>{children}</div>
}
