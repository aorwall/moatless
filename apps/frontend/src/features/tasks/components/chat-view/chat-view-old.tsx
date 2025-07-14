import type { Command } from '@moatless/types/types/command'
import type { Task } from '@moatless/types/types/task'
import { useParams } from '@tanstack/react-router'
import { ArrowDown } from 'lucide-react'
import React, { useCallback, useMemo, useRef } from 'react'
import { toast } from 'sonner'
import {
  useEventStream,
  useStreamingStatus,
  useTaskData,
  useWorkspaceStatus,
} from '@/features/tasks/contexts/task-providers'
import { useAutoScroll } from '@/features/tasks/hooks/use-auto-scroll'
import { useMessageCounting } from '@/features/tasks/hooks/use-message-counting'
import { useScrollManager } from '@/features/tasks/hooks/use-scroll-manager'
import { useSendMessage, useTaskCommands } from '@/features/tasks/hooks/use-tasks'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { Button } from '@/lib/components/ui/button'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { getErrorMessage } from '@/lib/utils/error'
import { ChatInput } from '../chat-input'
import { TaskMinifiedLastEventsDisplay } from '../event-viewer'
import { ChatHeader } from './chat-header'
import { ChatViewSkeleton } from './chat-view-skeleton'
import { CommandButtons } from './command-buttons'
import { TaskIteration } from './task-iteration'

function ChatViewComponent() {
  const params = useParams({ from: '/tasks/$taskId' })

  // Use specialized hooks from tri-context pattern
  const { taskId, status, iterations, repositoryId, isLoading: isLoadingTask } = useTaskData()
  const { events } = useEventStream()
  const { isStreaming } = useStreamingStatus()
  const { scrollRef } = useAutoScroll()
  const bottomRef = useRef<HTMLDivElement>(null)

  // Custom hooks for extracted functionality
  const { isAtBottom, scrollToBottom } = useScrollManager({ scrollRef, bottomRef })
  const { getMessageCountForIteration } = useMessageCounting({ events, isStreaming })

  // Get workspace status from context (doesn't block loading) - no longer needed for command filtering
  useWorkspaceStatus()

  // Fetch available commands for this task
  const { data: commandsData } = useTaskCommands(params.taskId, {
    enabled: !!params.taskId && status !== 'running',
  })

  // Task message sending hook
  const sendMessage = useSendMessage(params.taskId, {
    onError: (error) => {
      console.error('Failed to send message:', error)
      toast.error('Failed to send message', {
        description: getErrorMessage(error),
      })
    },
  })

  // Handle sending message to task - wrapped in useCallback for stable reference
  const handleSendMessage = useCallback(
    (
      _taskId: string,
      message: string,
      clearMessage: () => void,
      resume?: boolean,
      model?: string,
      filePaths?: string[],
      commandName?: string,
    ) => {
      sendMessage.mutate(
        { message, resume, model, filePaths, commandName },
        {
          onSuccess: () => {
            clearMessage()
          },
        },
      )
    },
    [sendMessage],
  )

  // Get last iteration for ChatInput
  const lastIteration =
    iterations && iterations.length > 0 ? iterations[iterations.length - 1] : null

  // Get available commands from the API response
  const availableCommands = useMemo(() => {
    if (!commandsData || status === 'running') return []
    return commandsData.commands
  }, [commandsData, status])

  // Handle command click
  const handleCommandClick = useCallback(
    (command: Command) => {
      handleSendMessage(
        params.taskId,
        command.prompt,
        () => {},
        true,
        undefined,
        undefined,
        command.name,
      )
    },
    [params.taskId, handleSendMessage],
  )

  // Show skeleton during initial load
  if (isLoadingTask || !taskId) {
    return <ChatViewSkeleton />
  }

  // Show minimal UI while events are loading for the first time
  const isLoadingEvents = isLoadingTask
  if (isLoadingEvents && !events.length && !iterations?.length) {
    return (
      <div className="h-full flex flex-col">
        <ChatHeader />
        <div className="flex-1 overflow-hidden p-4">
          <div className="bg-secondary rounded-lg p-4 max-w-[48rem] mx-auto">
            <div className="text-sm text-muted-foreground text-center">Loading task data...</div>
          </div>
        </div>
        <ChatInput
          taskId={params.taskId}
          taskStatus={status}
          onSendMessage={handleSendMessage}
          isLoading={true}
          className="p-4 border-t border-border"
        />
      </div>
    )
  }

  const isTaskRunning = status === 'running'

  // Check if last iteration has an error
  const lastIterationHasError =
    lastIteration && (lastIteration.status === 'error' || lastIteration.status === 'failed')

  return (
    <div className="h-full flex flex-col">
      <ChatHeader />
      <div className="flex-1 overflow-hidden relative">
        <ScrollArea className="h-full w-full" ref={scrollRef}>
          <div className="p-3 sm:p-4 lg:p-6 space-y-4 max-w-[48rem] mx-auto">
            {iterations?.map((iteration, index: number) => {
              const messageCount = getMessageCountForIteration(index, iteration.messageCount)
              const isLastIteration = index === (iterations?.length || 0) - 1

              return (
                <TaskIteration
                  key={iteration.iterationNumber || `iteration-${index}`}
                  iteration={iteration}
                  index={index}
                  messageCount={messageCount}
                  isLastIteration={isLastIteration}
                  status={status}
                />
              )
            })}

            {/* Show last three events when task is running */}
            {isTaskRunning && (
              <div className="mb-4">
                <TaskMinifiedLastEventsDisplay />
              </div>
            )}

            {/* Show available command buttons */}
            {!isTaskRunning && (
              <CommandButtons
                commands={availableCommands as Command[]}
                onCommandClick={handleCommandClick}
                disabled={sendMessage.isPending}
              />
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
      <ErrorBoundary componentName="Chat Input">
        <ChatInput
          data-testid="task-chat-input"
          taskId={params.taskId}
          task={
            iterations && repositoryId
              ? ({ iterations, repositoryId } as unknown as Task)
              : undefined
          }
          taskStatus={status}
          lastIterationHasError={lastIterationHasError || false}
          onSendMessage={handleSendMessage}
          isLoading={sendMessage.isPending}
          className="p-4 border-t border-border"
          maxHeight="150px"
        />
      </ErrorBoundary>
    </div>
  )
}

export const ChatView = React.memo(ChatViewComponent)
