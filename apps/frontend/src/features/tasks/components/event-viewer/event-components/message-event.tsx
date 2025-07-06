import type { Event } from '@moatless/shared/types/event'
import { Bot, User } from 'lucide-react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'

import { Progress } from '@/lib/components/ui/progress'
import { EventHeader } from './event-header'

interface MessageEventProps {
  event: Event
}

export function MessageEvent({ event }: MessageEventProps) {
  const payload = event.payload as Record<string, unknown>

  // Extract message object
  const message = payload.message as Record<string, unknown> | null
  if (!message) {
    return <div className="text-sm text-muted-foreground">No message content</div>
  }

  // Determine the role/type
  const role = payload.type || message.role || 'unknown'
  const isAssistant = role === 'assistant'
  const isUser = role === 'user'

  // Extract usage information for context tokens display
  const usage = message.usage as Record<string, unknown> | null
  const contextTokens = usage ? 
    (Number(usage.input_tokens || 0) + Number(usage.cache_read_input_tokens || 0)) : null

  // Extract text content
  let textContent = ''

  if (message.content && Array.isArray(message.content)) {
    // Extract text from content array
    for (const item of message.content) {
      if (
        item &&
        typeof item === 'object' &&
        'type' in item &&
        item.type === 'text' &&
        'text' in item
      ) {
        textContent += String(item.text || '')
      }
    }
  } else if ('content' in message && typeof message.content === 'string') {
    textContent = message.content
  } else if ('text' in message) {
    textContent = String(message.text || '')
  } else if ('message' in message) {
    textContent = String(message.message || '')
  }

  if (!textContent) {
    return <div className="text-sm text-muted-foreground">No text content</div>
  }

  return (
    <EventHeader event={event}>
      <div className="space-y-2">
        <div className="flex items-center gap-2">
          {isAssistant && <Bot className="h-4 w-4 text-muted-foreground" />}
          {isUser && <User className="h-4 w-4 text-muted-foreground" />}
          <span className="text-sm font-medium text-foreground">
            {isAssistant ? 'Assistant' : isUser ? 'User' : String(role)}
          </span>
          {contextTokens && contextTokens > 0 && (
            <div className="flex items-center gap-2">
              <span className="text-xs text-muted-foreground">
                {contextTokens.toLocaleString()}
              </span>
              <Progress 
                value={(contextTokens / 200000) * 100} 
                className="w-16 h-1.5" 
              />
            </div>
          )}
        </div>

        <div className="text-sm prose prose-sm dark:prose-invert max-w-none">
          <ReactMarkdown
            remarkPlugins={[remarkGfm]}
            components={{
              // Customize markdown components if needed
              p: ({ children }) => <p className="mb-2">{children}</p>,
              pre: ({ children }) => (
                <pre className="bg-muted rounded-md p-3 overflow-x-auto">{children}</pre>
              ),
              code: ({ children, ...props }) => {
                const inline = !(
                  'className' in props &&
                  typeof props.className === 'string' &&
                  props.className.includes('language-')
                )
                return inline ? (
                  <code className="bg-muted px-1 py-0.5 rounded text-xs">{children}</code>
                ) : (
                  <code {...props}>{children}</code>
                )
              },
            }}
          >
            {textContent}
          </ReactMarkdown>
        </div>
      </div>
    </EventHeader>
  )
}
