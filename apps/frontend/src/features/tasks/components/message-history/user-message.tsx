import type { UserMessage } from '@moatless/types'
import { User } from 'lucide-react'

interface UserMessageProps {
  message: UserMessage
}

export function UserMessageComponent({ message }: UserMessageProps) {
  return (
    <div className="flex justify-end">
      <div className="bg-primary/10 text-primary-foreground rounded-lg p-3 max-w-[80%] shadow-sm">
        <div className="flex items-center gap-2 mb-2">
          <User className="h-4 w-4" />
          <span className="text-sm font-medium">You</span>
          <span className="text-xs text-muted-foreground">
            {new Date(message.timestamp).toLocaleTimeString()}
          </span>
        </div>
        <div className="text-sm whitespace-pre-wrap break-words">{message.content}</div>
      </div>
    </div>
  )
}
