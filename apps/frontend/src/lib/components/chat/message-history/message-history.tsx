import type {
  AssistantMessage,
  AssistantMessageContent,
  Message,
  UserMessage,
} from '@moatless/types'
import { ArrowDown, ChevronDown, Terminal } from 'lucide-react'
import React, { useCallback, useEffect, useRef, useState } from 'react'
import { groupMessages } from '@/features/tasks/utils/group-messages.ts'
import { Button } from '@/lib/components/ui/button.tsx'
import { useAutoScroll } from '@/lib/hooks/use-auto-scroll'
import { cn } from '@/lib/utils'
import { getErrorMessage } from '@/lib/utils/error.ts'
import { AssistantMessageComponent } from './assistant-message.tsx'
import { MessageHistorySkeleton } from './message-history-skeleton.tsx'
import { ThinkingIndicator } from './thinking-indicator.tsx'
import { UserMessageComponent } from './user-message.tsx'

interface MessageHistoryProps {
  messages?: Message[]
  isLoading?: boolean
  error?: Error | null
  className?: string
  isRunning?: boolean
}

interface ToolCallGroupProps {
  messages: AssistantMessage[]
  expandedToolCalls: Set<string>
  onToggleToolCall: (toolUseId: string) => void
}

function ToolCallGroup({ messages, expandedToolCalls, onToggleToolCall }: ToolCallGroupProps) {
  const [groupExpanded, setGroupExpanded] = useState(false)

  // Extract all tool calls from all messages
  const allToolCalls: {
    message: AssistantMessage
    content: AssistantMessageContent
  }[] = []
  for (const message of messages) {
    if (typeof message.content !== 'string') {
      for (const content of message.content as AssistantMessageContent[]) {
        if (content.type === 'tool_use') {
          allToolCalls.push({ message, content })
        }
      }
    }
  }

  // Count tools by name
  const toolCounts = allToolCalls.reduce(
    (acc, { content }) => {
      const name = content.tool_name || 'Unknown'
      acc[name] = (acc[name] || 0) + 1
      return acc
    },
    {} as Record<string, number>,
  )

  const totalCalls = allToolCalls.length
  const toolSummary = Object.entries(toolCounts)
    .sort(([, a], [, b]) => b - a)
    .map(([name, count]) => (count > 1 ? `${count} ${name}` : name))
    .join(', ')

  return (
    <div className="border border-border/50 rounded-lg overflow-hidden w-full max-w-full">
      <Button
        variant="ghost"
        size="sm"
        onClick={() => setGroupExpanded(!groupExpanded)}
        className="w-full justify-start p-2 h-auto font-mono text-xs hover:bg-muted/50"
      >
        <div className="flex items-center gap-2 w-full">
          {groupExpanded ? (
            <ChevronDown className="h-3 w-3  text-muted-foreground" />
          ) : (
            <Terminal className="h-3 w-3 text-muted-foreground" />
          )}
          <span className="text-muted-foreground">
            {totalCalls} tool {totalCalls === 1 ? 'call' : 'calls'} ({toolSummary})
          </span>
        </div>
      </Button>

      {groupExpanded && (
        <div className="p-2 space-y-2 border-t border-border/50">
          {messages.map((message) => (
            <AssistantMessageComponent
              key={message.id}
              message={message}
              expandedToolCalls={expandedToolCalls}
              onToggleToolCall={onToggleToolCall}
            />
          ))}
        </div>
      )}
    </div>
  )
}

function MessageHistoryComponent({
  messages,
  isLoading = false,
  error = null,
  className,
  isRunning = false,
}: MessageHistoryProps) {
  const [expandedToolCalls, setExpandedToolCalls] = useState<Set<string>>(new Set())
  const [initialMessageCount, setInitialMessageCount] = useState<number | null>(null)

  // Initialize auto-scroll with proper options
  const { scrollRef, isAtBottom, scrollToBottom } = useAutoScroll({
    scrollOnMount: true,
    dependencies: [messages],
  })
  const bottomRef = useRef<HTMLDivElement>(null)
  const hasInitiallyScrolled = useRef(false)

  // Track initial message count on mount
  useEffect(() => {
    if (messages && initialMessageCount === null) {
      setInitialMessageCount(messages.length)
    }
  }, [messages, initialMessageCount])

  // Ensure scroll to bottom when messages first load
  useEffect(() => {
    if (messages && messages.length > 0 && !hasInitiallyScrolled.current) {
      // Small delay to ensure DOM is fully rendered
      const timeoutId = setTimeout(() => {
        scrollToBottom()
        hasInitiallyScrolled.current = true
      }, 100)
      return () => clearTimeout(timeoutId)
    }
  }, [messages, scrollToBottom])

  // Toggle tool call expansion
  const toggleToolCallExpansion = useCallback((toolUseId: string) => {
    setExpandedToolCalls((prev) => {
      const newSet = new Set(prev)
      if (newSet.has(toolUseId)) {
        newSet.delete(toolUseId)
      } else {
        newSet.add(toolUseId)
      }
      return newSet
    })
  }, [])

  // Group consecutive assistant messages with only tool calls
  const groupedMessages = React.useMemo(() => {
    if (!messages) {
      return []
    }
    return groupMessages(messages, initialMessageCount)
  }, [messages, initialMessageCount])

  // Show skeleton during initial load
  if (isLoading && (!messages || messages.length === 0)) {
    return <MessageHistorySkeleton />
  }

  // Handle error state
  if (error) {
    return (
      <div className={cn('flex-1 overflow-hidden min-h-0', className)}>
        <div className="h-full flex flex-col p-4">
          <div className="bg-destructive/10 border border-destructive/20 rounded-lg p-4">
            <div className="text-sm text-destructive text-center">
              Failed to load messages: {getErrorMessage(error)}
            </div>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className={cn('flex-1 overflow-hidden min-h-0', className)}>
      <div className="h-full relative">
        <div
          ref={scrollRef}
          className="h-full w-full overflow-y-auto [&::-webkit-scrollbar]:w-2.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-border [&::-webkit-scrollbar-thumb]:rounded-full [&::-webkit-scrollbar-thumb]:transition-colors"
          style={{
            scrollbarWidth: 'thin',
            scrollbarColor: 'hsl(var(--border)) transparent',
          }}
        >
          <div className="p-4 space-y-4 w-full max-w-full">
            {groupedMessages.map((item) => {
              if (Array.isArray(item)) {
                // This is a group of tool-only messages
                const groupKey = item.map((m) => m.id).join('-')
                return (
                  <ToolCallGroup
                    key={groupKey}
                    messages={item}
                    expandedToolCalls={expandedToolCalls}
                    onToggleToolCall={toggleToolCallExpansion}
                  />
                )
              }
              if (item.role === 'user') {
                return <UserMessageComponent key={item.id} message={item as UserMessage} />
              }
              return (
                <AssistantMessageComponent
                  key={item.id}
                  message={item as AssistantMessage}
                  expandedToolCalls={expandedToolCalls}
                  onToggleToolCall={toggleToolCallExpansion}
                />
              )
            })}

            {/* Empty state */}
            {(!messages || (messages && messages.length === 0)) && !isLoading && (
              <div className="text-center text-muted-foreground py-8">
                <p>No messages yet. Start a conversation!</p>
              </div>
            )}

            {/* Thinking indicator */}
            {isRunning && <ThinkingIndicator />}

            <div ref={bottomRef} />
          </div>
        </div>

        {/* Scroll to bottom indicator */}
        {!isAtBottom && (
          <div className="absolute bottom-4 right-4 z-10">
            <Button
              variant="outline"
              size="sm"
              onClick={scrollToBottom}
              className="shadow-lg bg-background/80 backdrop-blur-sm border-border/50 hover:bg-background/90 flex items-center gap-2"
            >
              <ArrowDown className="h-4 w-4" />
              Scroll to bottom
            </Button>
          </div>
        )}
      </div>
    </div>
  )
}

export const MessageHistory = React.memo(MessageHistoryComponent)
