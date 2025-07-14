import type { Command } from '@moatless/types/types/command'
import type { Task } from '@moatless/types/types/task'
import { useParams } from '@tanstack/react-router'
import React, { useCallback, useMemo } from 'react'
import { toast } from 'sonner'
import { useTaskData, useWorkspaceStatus } from '@/features/tasks/contexts/task-providers'
import { useSendMessage, useTaskCommands } from '@/features/tasks/hooks/use-tasks'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { getErrorMessage } from '@/lib/utils/error'
import { ChatInput } from '../chat-input'
import { MessageHistory } from '../message-history'
import { ChatHeader } from './chat-header'
import { CommandButtons } from './command-buttons'

interface ChatViewProps {
  className?: string
}

function ChatViewComponent({ className }: ChatViewProps) {
  const params = useParams({ from: '/tasks/$taskId' })

  // Use specialized hooks from tri-context pattern
  const { status, iterations, repositoryId } = useTaskData()

  // Get workspace status from context (doesn't block loading)
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

  // Check if last iteration has an error
  const lastIterationHasError =
    lastIteration && (lastIteration.status === 'error' || lastIteration.status === 'failed')

  const isTaskRunning = status === 'running'

  return (
    <div className={`h-full flex flex-col ${className || ''}`}>
      {/* Chat Header */}
      <ChatHeader />

      {/* Message History Component */}
      <div className="flex-1 overflow-hidden">
        <ErrorBoundary componentName="Message History">
          <MessageHistory />
        </ErrorBoundary>
      </div>

      {/* Command Buttons - shown when task is not running */}
      {!isTaskRunning && availableCommands.length > 0 && (
        <div className="border-t border-border p-4">
          <CommandButtons
            commands={availableCommands as Command[]}
            onCommandClick={handleCommandClick}
            disabled={sendMessage.isPending}
          />
        </div>
      )}

      {/* Chat Input Component */}
      <div className="border-t border-border">
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
            className="p-4"
            maxHeight="150px"
          />
        </ErrorBoundary>
      </div>
    </div>
  )
}

export const ChatView = React.memo(ChatViewComponent)
