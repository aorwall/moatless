import type { AssistantMessage, Message, UserMessage } from '@moatless/types'
import { useParams } from '@tanstack/react-router'
import { ArrowDown } from 'lucide-react'
import React, { useCallback, useRef, useState } from 'react'
import { useTaskData } from '@/features/tasks/contexts/task-providers'
import { useAutoScroll } from '@/features/tasks/hooks/use-auto-scroll'
import { useScrollManager } from '@/features/tasks/hooks/use-scroll-manager'
import { useTaskMessages } from '@/features/tasks/hooks/use-tasks'
import { Button } from '@/lib/components/ui/button'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { getErrorMessage } from '@/lib/utils/error'
import { AssistantMessageComponent } from './assistant-message'
import { MessageHistorySkeleton } from './message-history-skeleton'
import { UserMessageComponent } from './user-message'

interface MessageHistoryProps {
  className?: string
}

function MessageHistoryComponent({ className }: MessageHistoryProps) {
  const params = useParams({ from: '/tasks/$taskId' })
  const [expandedToolCalls, setExpandedToolCalls] = useState<Set<string>>(new Set())

  // Use specialized hooks from tri-context pattern
  const { taskId, isLoading: isLoadingTask } = useTaskData()
  const { scrollRef } = useAutoScroll()
  const bottomRef = useRef<HTMLDivElement>(null)

  // Custom hooks for extracted functionality
  const { isAtBottom, scrollToBottom } = useScrollManager({ scrollRef, bottomRef })

  // Fetch messages for this task
  const {
    data: messages,
    isLoading: isLoadingMessages,
    error: messagesError,
  } = useTaskMessages(params.taskId, {
    enabled: !!params.taskId,
  }) as { data: Message[] | undefined; isLoading: boolean; error: unknown }

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

  // Show skeleton during initial load
  if (isLoadingTask || !taskId) {
    return <MessageHistorySkeleton />
  }

  // Handle error state
  if (messagesError) {
    return (
      <div className="h-full flex flex-col">
        <div className="flex-1 overflow-hidden p-4">
          <div className="bg-destructive/10 border border-destructive/20 rounded-lg p-4 max-w-[48rem] mx-auto">
            <div className="text-sm text-destructive text-center">
              Failed to load messages: {getErrorMessage(messagesError)}
            </div>
          </div>
        </div>
      </div>
    )
  }

  // Show minimal UI while messages are loading for the first time
  if (isLoadingMessages && (!messages || messages.length === 0)) {
    return (
      <div className="h-full flex flex-col">
        <div className="flex-1 overflow-hidden p-4">
          <div className="bg-secondary rounded-lg p-4 max-w-[48rem] mx-auto">
            <div className="text-sm text-muted-foreground text-center">Loading messages...</div>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className={`h-full flex flex-col ${className || ''}`}>
      <div className="flex-1 overflow-hidden relative">
        <ScrollArea className="h-full w-full" ref={scrollRef}>
          <div className="p-3 sm:p-4 lg:p-6 space-y-4 max-w-[48rem] mx-auto">
            {messages &&
              messages.map((message: Message) => {
                if (message.role === 'user') {
                  return <UserMessageComponent key={message.id} message={message as UserMessage} />
                } else {
                  return (
                    <AssistantMessageComponent
                      key={message.id}
                      message={message as AssistantMessage}
                      expandedToolCalls={expandedToolCalls}
                      onToggleToolCall={toggleToolCallExpansion}
                    />
                  )
                }
              })}

            {/* Empty state */}
            {(!messages || (messages && messages.length === 0)) && !isLoadingMessages && (
              <div className="text-center text-muted-foreground py-8">
                <p>No messages yet. Start a conversation!</p>
              </div>
            )}

            <div ref={bottomRef} />
          </div>
        </ScrollArea>

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
