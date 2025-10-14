import type { UserMessage } from '@moatless/types'
import { Message, MessageContent } from '@/components/ai-elements/message'
import { FileAttachments } from '@/lib/components/chat/chat-input/file-attachments'

interface UserMessageProps {
  message: UserMessage
}

export function UserMessageComponent({ message }: UserMessageProps) {
  return (
    <Message from="user">
      <MessageContent variant="flat">
        <div className="max-w-none break-words">{message.content}</div>
        {message.attachments && message.attachments.length > 0 && (
          <FileAttachments files={message.attachments} />
        )}
      </MessageContent>
    </Message>
  )
}
