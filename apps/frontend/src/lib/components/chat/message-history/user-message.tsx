import type { UserMessage } from '@moatless/types'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { FileAttachments } from '@/lib/components/chat/chat-input/file-attachments'

interface UserMessageProps {
  message: UserMessage
}

export function UserMessageComponent({ message }: UserMessageProps) {
  return (
    <div className="flex justify-end w-full max-w-full">
      <div className="flex flex-col gap-2 max-w-[80%]">
        <div className="bg-muted rounded-lg p-3 text-sm prose prose-sm dark:prose-invert prose-max-w-none break-words">
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{message.content}</ReactMarkdown>
        </div>
        {message.attachments && message.attachments.length > 0 && (
          <FileAttachments files={message.attachments} />
        )}
      </div>
    </div>
  )
}
