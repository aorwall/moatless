import type { UserMessage } from '@moatless/types'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'

interface UserMessageProps {
  message: UserMessage
}

export function UserMessageComponent({ message }: UserMessageProps) {
  return (
    <div className="flex justify-end w-full max-w-full">
      <div className="bg-muted rounded-lg p-3 max-w-[80%] text-sm prose prose-sm dark:prose-invert prose-max-w-none break-words">
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{message.content}</ReactMarkdown>
      </div>
    </div>
  )
}
