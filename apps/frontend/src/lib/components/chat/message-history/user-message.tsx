import type { UserMessage } from '@moatless/types'
import { Message, MessageContent } from '@/components/ai-elements/message'
import { ContextItems } from '@/lib/components/chat/chat-input/context-items'
import { FileAttachments } from '@/lib/components/chat/chat-input/file-attachments'

interface UserMessageProps {
  message: UserMessage
}

export function UserMessageComponent({ message }: UserMessageProps) {
  return (
    <Message from="user">
      <MessageContent variant="flat">
        <div className="max-w-none break-words">{message.content}</div>
        {message.contexts && message.contexts.length > 0 && (
          <ContextItems contexts={message.contexts} />
        )}
        {message.attachments && message.attachments.length > 0 && (
          <FileAttachments files={message.attachments} />
        )}
      </MessageContent>
    </Message>
  )
}
