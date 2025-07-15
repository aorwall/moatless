import type { UserMessage } from '@moatless/types'

interface UserMessageProps {
  message: UserMessage
}

export function UserMessageComponent({ message }: UserMessageProps) {
  return (
    <div className="flex justify-end">
      <div className="bg-secondary rounded-lg p-3 max-w-[80%] shadow-sm text-sm whitespace-pre-wrap break-words">
        {message.content}
      </div>
    </div>
  )
}
