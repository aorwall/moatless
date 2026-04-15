import type { EnvironmentStatus, UiContentBlock, UiMessage } from '@moatless/api-client'
import { ArrowDown, ChevronDown, Terminal } from 'lucide-react'
import React, { useCallback, useEffect, useState } from 'react'
import { StickToBottom, useStickToBottomContext } from 'use-stick-to-bottom'
import {
  groupMessages,
  groupSubagentMessages,
  type UiMessageWithSubagents,
} from '@/features/tasks/utils/group-messages.ts'
import { Button } from '@/lib/components/ui/button.tsx'
import { cn } from '@/lib/utils'
import { getErrorMessage } from '@/lib/utils/error.ts'
import { AssistantMessageComponent } from './assistant-message.tsx'
import { ChannelMessageComponent } from './channel-message.tsx'
import { EnvironmentIndicator } from './environment-indicator.tsx'
import { MessageHistorySkeleton } from './message-history-skeleton.tsx'
import { ResultMessageComponent } from './result-message.tsx'
import { SystemMessageComponent } from './system-message.tsx'
import { ThinkingIndicator } from './thinking-indicator.tsx'
import { UserMessageComponent } from './user-message.tsx'

interface MessageHistoryProps {
  messages?: UiMessage[]
  isLoading?: boolean
  error?: Error | null
  className?: string
  isRunning?: boolean
  environmentStatus?: EnvironmentStatus
  environmentError?: string
  onPreviewUrlClick?: (path: string) => void
}

interface ToolCallGroupProps {
  messages: UiMessageWithSubagents[]
  expandedToolCalls: Set<string>
  onToggleToolCall: (toolUseId: string) => void
}

function ScrollToBottomButton() {
  const { isAtBottom, scrollToBottom } = useStickToBottomContext()

  if (isAtBottom) {
    return null
  }

  return (
    <div className="absolute bottom-4 right-4 z-10">
      <Button
        variant="outline"
        size="sm"
        onClick={() => scrollToBottom()}
        className="shadow-lg bg-background/80 backdrop-blur-sm border-border/50 hover:bg-background/90 flex items-center gap-2"
      >
        <ArrowDown className="h-4 w-4" />
        Scroll to bottom
      </Button>
    </div>
  )
}

// Type for TodoWrite tool input
interface TodoItem {
  content: string
  status: 'pending' | 'in_progress' | 'completed'
  activeForm: string
}

// Helper to find label from todo items
function findTodoLabel(todos: TodoItem[]): string | null {
  if (!Array.isArray(todos) || todos.length === 0) {
    return null
  }

  // Find last completed item
  for (let i = todos.length - 1; i >= 0; i--) {
    if (todos[i].status === 'completed' && todos[i].activeForm) {
      return todos[i].activeForm
    }
  }

  // Fall back to last in_progress item
  for (let i = todos.length - 1; i >= 0; i--) {
    if (todos[i].status === 'in_progress' && todos[i].activeForm) {
      return todos[i].activeForm
    }
  }

  return null
}

// Helper to extract TodoWrite tool input from message content
function extractTodoWriteInput(content: UiContentBlock[]): TodoItem[] | null {
  for (const item of content) {
    if (item.type === 'tool_use' && item.toolName === 'TodoWrite') {
      const toolInput = item.toolInput as { todos?: TodoItem[] } | undefined
      return toolInput?.todos || null
    }
  }
  return null
}

// Helper to extract label from TodoWrite tool calls
function getTodoGroupLabel(messages: UiMessage[]): string | null {
  // Find the last TodoWrite tool call in the group
  for (let i = messages.length - 1; i >= 0; i--) {
    const message = messages[i]
    const todos = extractTodoWriteInput(message.content)

    if (todos) {
      return findTodoLabel(todos)
    }
  }

  return null
}

function ToolCallGroup({ messages, expandedToolCalls, onToggleToolCall }: ToolCallGroupProps) {
  const [groupExpanded, setGroupExpanded] = useState(false)

  // Extract all tool calls from all messages
  const allToolCalls: {
    message: UiMessage
    content: UiContentBlock
  }[] = []
  for (const message of messages) {
    for (const content of message.content) {
      if (content.type === 'tool_use') {
        allToolCalls.push({ message, content })
      }
    }
  }

  const totalCalls = allToolCalls.length

  // If only one tool call, render it directly without grouping
  if (totalCalls === 1) {
    return (
      <AssistantMessageComponent
        key={messages[0].uuid}
        message={messages[0]}
        expandedToolCalls={expandedToolCalls}
        onToggleToolCall={onToggleToolCall}
      />
    )
  }

  // Get smart label for TodoWrite groups
  const todoLabel = getTodoGroupLabel(messages)

  // Extract unique tool names for display when no todo label
  const uniqueToolNames = Array.from(new Set(allToolCalls.map((tc) => tc.content.toolName))).join(
    ', ',
  )

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
            {todoLabel ? (
              <>
                {todoLabel} ({totalCalls})
              </>
            ) : (
              <>
                {totalCalls} tool {totalCalls === 1 ? 'call' : 'calls'} ({uniqueToolNames})
              </>
            )}
          </span>
        </div>
      </Button>

      {groupExpanded && (
        <div className="p-2 space-y-2 border-t border-border/50">
          {messages.map((message) => (
            <AssistantMessageComponent
              key={message.uuid}
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
  environmentStatus,
  environmentError,
  onPreviewUrlClick,
}: MessageHistoryProps) {
  const [expandedToolCalls, setExpandedToolCalls] = useState<Set<string>>(new Set())
  const [initialMessageCount, setInitialMessageCount] = useState<number | null>(null)

  // Track initial message count on mount
  useEffect(() => {
    if (messages && initialMessageCount === null) {
      setInitialMessageCount(messages.length)
    }
  }, [messages, initialMessageCount])

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

  // Group subagent messages under parent Agent tool calls, then group consecutive tool calls
  const groupedMessages = React.useMemo(() => {
    if (!messages) {
      return []
    }
    // First, group subagent messages under their parent Agent tool calls
    const messagesWithSubagents = groupSubagentMessages(messages)
    // Then, group consecutive assistant messages with only tool calls
    return groupMessages(messagesWithSubagents, initialMessageCount)
  }, [messages, initialMessageCount])

  // Show skeleton during initial load
  if (isLoading && (!messages || messages.length === 0)) {
    return <MessageHistorySkeleton />
  }

  // Handle error state
  if (error) {
    return (
      <div
        className={cn('flex-1 min-h-0 flex flex-col items-center justify-center p-4', className)}
      >
        <div className="bg-destructive/10 border border-destructive/20 rounded-lg p-4">
          <div className="text-sm text-destructive text-center">
            Failed to load messages: {getErrorMessage(error)}
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className={cn('flex-1 overflow-hidden min-h-0', className)}>
      <div className="h-full relative">
        <StickToBottom
          initial="instant"
          className="h-full w-full overflow-y-auto overscroll-contain [&::-webkit-scrollbar]:w-2.5 [&::-webkit-scrollbar-track]:bg-transparent [&::-webkit-scrollbar-thumb]:bg-border [&::-webkit-scrollbar-thumb]:rounded-full [&::-webkit-scrollbar-thumb]:transition-colors"
          style={{
            scrollbarWidth: 'thin',
            scrollbarColor: 'hsl(var(--border)) transparent',
          }}
        >
          <StickToBottom.Content>
            <div className="px-4 pt-[74px] pb-[var(--chat-input-height,120px)] space-y-4 w-full max-w-full">
              {groupedMessages.map((item) => {
                if (Array.isArray(item)) {
                  // This is a group of tool-only messages
                  const groupKey = item.map((m) => m.uuid).join('-')
                  return (
                    <ToolCallGroup
                      key={groupKey}
                      messages={item}
                      expandedToolCalls={expandedToolCalls}
                      onToggleToolCall={toggleToolCallExpansion}
                    />
                  )
                }
                if (item.messageType === 'result') {
                  return <ResultMessageComponent key={item.uuid} message={item} />
                }
                if (item.role === 'user' && item.channel) {
                  return <ChannelMessageComponent key={item.uuid} message={item} />
                }
                if (item.role === 'user') {
                  return (
                    <UserMessageComponent
                      key={item.uuid}
                      message={item}
                      onPreviewUrlClick={onPreviewUrlClick}
                    />
                  )
                }
                if (item.role === 'system') {
                  return <SystemMessageComponent key={item.uuid} message={item} />
                }
                return (
                  <AssistantMessageComponent
                    key={item.uuid}
                    message={item}
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

              {/* Status indicator */}
              {environmentStatus === 'initializing' && (
                <EnvironmentIndicator status="initializing" />
              )}
              {environmentStatus === 'error' && (
                <EnvironmentIndicator status="error" errorMessage={environmentError} />
              )}
              {isRunning && environmentStatus !== 'initializing' && <ThinkingIndicator />}
            </div>
          </StickToBottom.Content>

          <ScrollToBottomButton />
        </StickToBottom>
      </div>
    </div>
  )
}

export const MessageHistory = React.memo(MessageHistoryComponent)
