import type {
  AssistantMessage,
  AssistantMessageContent,
  Message,
  UserMessage,
} from '@moatless/types'
import { useParams } from '@tanstack/react-router'
import { ArrowDown, ChevronDown, Terminal } from 'lucide-react'
import React, { useCallback, useEffect, useRef, useState } from 'react'
import { useTaskData } from '@/features/tasks/contexts/task-providers'
import { useChatState } from '@/features/tasks/contexts/task-ui-provider'
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

interface ToolCallGroupProps {
  messages: AssistantMessage[]
  expandedToolCalls: Set<string>
  onToggleToolCall: (toolUseId: string) => void
}

function ToolCallGroup({ messages, expandedToolCalls, onToggleToolCall }: ToolCallGroupProps) {
  const [groupExpanded, setGroupExpanded] = useState(false)

  // Extract all tool calls from all messages
  const allToolCalls: Array<{ message: AssistantMessage; content: AssistantMessageContent }> = []
  messages.forEach((message) => {
    if (typeof message.content !== 'string') {
      message.content.forEach((content: AssistantMessageContent) => {
        if (content.type === 'tool_use') {
          allToolCalls.push({ message, content })
        }
      })
    }
  })

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

function MessageHistoryComponent({ className }: MessageHistoryProps) {
  const params = useParams({ from: '/tasks/$taskId' })
  const [expandedToolCalls, setExpandedToolCalls] = useState<Set<string>>(new Set())
  const [initialMessageCount, setInitialMessageCount] = useState<number | null>(null)

  // Use specialized hooks from tri-context pattern
  const { taskId, isLoading: isLoadingTask } = useTaskData()
  const { chatState, updateChatState } = useChatState()

  // Fetch messages for this task
  const {
    data: messages,
    isLoading: isLoadingMessages,
    error: messagesError,
  } = useTaskMessages(params.taskId, {
    enabled: !!params.taskId,
  }) as { data: Message[] | undefined; isLoading: boolean; error: unknown }

  // Initialize auto-scroll with proper options
  const { scrollRef } = useAutoScroll({
    // Scroll on mount only if there's no saved scroll position
    scrollOnMount: chatState.scrollPosition === undefined,
    dependencies: [messages], // Re-evaluate scroll when messages change
  })
  const bottomRef = useRef<HTMLDivElement>(null)

  // Track initial message count on mount
  useEffect(() => {
    if (messages && initialMessageCount === null) {
      setInitialMessageCount(messages.length)
    }
  }, [messages, initialMessageCount])

  // Custom hooks for extracted functionality
  const { isAtBottom, scrollToBottom } = useScrollManager({ scrollRef, bottomRef })

  // Update chat state when scroll position changes
  useEffect(() => {
    updateChatState({ isAtBottom })
  }, [isAtBottom, updateChatState])

  // Fix ScrollArea viewport styles
  useEffect(() => {
    if (scrollRef.current) {
      const viewport = scrollRef.current.querySelector(
        '[data-slot="scroll-area-viewport"]',
      ) as HTMLElement
      if (viewport) {
        viewport.style.minWidth = '0'
        viewport.style.width = '100%'
        viewport.style.maxWidth = '100%'
        viewport.style.display = 'block'
        viewport.style.tableLayout = 'auto'
      }
    }
  }, [scrollRef])

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
    if (!messages) return []

    const groups: Array<Message | AssistantMessage[]> = []
    let currentToolGroup: AssistantMessage[] = []
    const shouldGroupMessages = initialMessageCount !== null

    messages.forEach((message, index) => {
      if (message.role === 'user') {
        // If we have accumulated tool messages, push them as a group (only if more than 1)
        if (currentToolGroup.length > 1) {
          groups.push(currentToolGroup)
        } else if (currentToolGroup.length === 1) {
          groups.push(currentToolGroup[0])
        }
        currentToolGroup = []
        groups.push(message)
      } else {
        const assistantMsg = message as AssistantMessage
        // Check if this message contains only tool calls (no text content)
        const hasOnlyToolCalls =
          typeof assistantMsg.content !== 'string' &&
          Array.isArray(assistantMsg.content) &&
          assistantMsg.content.length > 0 &&
          assistantMsg.content.every((c: AssistantMessageContent) => c.type === 'tool_use')

        // Only group messages that were present on mount OR new messages if view is open
        const shouldGroup =
          shouldGroupMessages &&
          (index < (initialMessageCount || 0) || initialMessageCount === messages.length)

        if (hasOnlyToolCalls && shouldGroup) {
          currentToolGroup.push(assistantMsg)
        } else {
          // This message has text content or shouldn't be grouped
          // Push any accumulated tool group first (only if more than 1 message)
          if (currentToolGroup.length > 1) {
            groups.push(currentToolGroup)
          } else if (currentToolGroup.length === 1) {
            // Single tool call message - don't group
            groups.push(currentToolGroup[0])
          }
          currentToolGroup = []
          groups.push(assistantMsg)
        }
      }
    })

    // Push any remaining tool group (only if more than 1 message)
    if (currentToolGroup.length > 1) {
      groups.push(currentToolGroup)
    } else if (currentToolGroup.length === 1) {
      // Single tool call message - don't group
      groups.push(currentToolGroup[0])
    }

    return groups
  }, [messages, initialMessageCount])

  // Show skeleton during initial load
  if (isLoadingTask || !taskId) {
    return <MessageHistorySkeleton />
  }

  // Handle error state
  if (messagesError) {
    return (
      <div className={`h-full flex flex-col ${className || ''}`}>
        <div className="flex-1 overflow-hidden p-4">
          <div className="bg-destructive/10 border border-destructive/20 rounded-lg p-4">
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
      <div className={`h-full flex flex-col ${className || ''}`}>
        <div className="flex-1 overflow-hidden p-4">
          <div className="bg-secondary rounded-lg p-4">
            <div className="text-sm text-muted-foreground text-center">Loading messages...</div>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className={`h-full flex flex-col ${className || ''}`}>
      <div className="flex-1 overflow-hidden relative">
        <ScrollArea className="h-full w-full scroll-area-constrained" ref={scrollRef}>
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
