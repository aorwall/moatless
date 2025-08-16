import type { ClaudeModel, Command, UploadFileResponse } from '@moatless/types'
import { CLAUDE_MODELS } from '@moatless/types/constants'
import { useParams } from '@tanstack/react-router'
import React, { useCallback, useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useIsAgentRunning, useTaskData } from '@/features/tasks/contexts/task-providers'
import { useSendMessage, useStopAgent, useTaskCommands } from '@/features/tasks/hooks/use-tasks'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { getErrorMessage } from '@/lib/utils/error'
import { ChatInput } from '../chat-input/chat-input'
import {
  PopoverSelector,
  SimpleListPopoverContent,
  useAgentModeSelector,
  useModelSelector,
} from '../chat-input/selectors'
import { MessageHistory } from '../message-history'
import { CommandButtons } from './command-buttons'

interface ChatViewProps {
  className?: string
}

function ChatViewComponent({ className }: ChatViewProps) {
  const params = useParams({ from: '/tasks/$taskId' })

  // Use specialized hooks from tri-context pattern
  const { status, iterations, repositoryId } = useTaskData()
  const isTaskRunning = useIsAgentRunning()

  // Local state for chat values
  const [message, setMessage] = useState('')
  const [uploadedFiles, setUploadedFiles] = useState<UploadFileResponse[]>([])
  const [resumeConversation, setResumeConversation] = useState(true)

  // Derive initial values from last iteration
  const lastIteration =
    iterations && iterations.length > 0 ? iterations[iterations.length - 1] : null
  const lastIterationModel = lastIteration?.model as ClaudeModel
  const lastIterationAgentMode = lastIteration?.agentMode as 'edit' | 'plan'

  // Initialize selectors with values from last iteration or defaults
  const modelSelector = useModelSelector(lastIterationModel || CLAUDE_MODELS.SONNET_4)
  const agentModeSelector = useAgentModeSelector(lastIterationAgentMode || 'edit')

  // Clear state when task changes
  // biome-ignore lint/correctness/useExhaustiveDependencies: taskId is from params and we want to reset state when it changes
  useEffect(() => {
    setMessage('')
    setUploadedFiles([])
  }, [params.taskId])

  // Stop agent mutation
  const stopAgent = useStopAgent(params.taskId)

  // Fetch available commands for this task
  const { data: commandsData } = useTaskCommands(params.taskId, {
    enabled: !!params.taskId && !isTaskRunning,
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

  // Handle sending message
  const handleChatSubmit = useCallback(() => {
    // If running, stop the agent instead of sending message
    if (isTaskRunning) {
      stopAgent.mutate()
      return
    }

    if (!message.trim()) {
      return
    }

    // Send message with file paths
    if (!sendMessage.isPending) {
      const filePaths = uploadedFiles.map((f) => f.absolutePath)

      sendMessage.mutate(
        {
          message: message.trim(),
          resume: resumeConversation,
          model: modelSelector.value as ClaudeModel,
          filePaths,
          agentMode: agentModeSelector.value,
        },
        {
          onSuccess: () => {
            // Clear state after successful send
            setMessage('')
            setUploadedFiles([])
          },
        },
      )
    }
  }, [
    isTaskRunning,
    stopAgent,
    sendMessage,
    message,
    uploadedFiles,
    resumeConversation,
    modelSelector.value,
    agentModeSelector.value,
  ])

  // Handle sending message to task for commands - wrapped in useCallback for stable reference
  const handleSendMessage = useCallback(
    (
      _taskId: string,
      messageText: string,
      clearMessageCallback: () => void,
      resume?: boolean,
      model?: string,
      filePaths?: string[],
      commandName?: string,
    ) => {
      sendMessage.mutate(
        { message: messageText, resume, model, filePaths, commandName },
        {
          onSuccess: () => {
            clearMessageCallback()
          },
        },
      )
    },
    [sendMessage],
  )

  // Get available commands from the API response
  const availableCommands = useMemo(() => {
    if (!commandsData || isTaskRunning) return []
    return commandsData.commands
  }, [commandsData, isTaskRunning])

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

  // Helper functions for ChatInput
  const getButtonText = () => {
    if (isTaskRunning) {
      return 'Running'
    }
    if (status === 'completed' || status === 'failed') {
      return 'Task Completed'
    }
    return sendMessage.isPending ? 'Sending...' : 'Send'
  }

  const getPlaceholderText = () => {
    if (isTaskRunning) {
      return 'Task is running... Type your next request'
    }
    if (status === 'completed' || status === 'failed') {
      return `Task is ${status}. Type a new request to continue`
    }
    return 'Type a new request... (use @ to reference files)'
  }

  // Check if last iteration has an error
  const lastIterationHasError = lastIteration && lastIteration.status === 'failed'

  // Disable sending based on conditions
  const isSendDisabled = sendMessage.isPending || (!isTaskRunning && !message.trim())

  return (
    <div className={`h-full flex flex-col ${className || ''}`}>
      {/* Message History Component - takes up available space and handles overflow */}
      <div className="flex-1 overflow-hidden min-h-0">
        <ErrorBoundary componentName="Message History">
          <MessageHistory className="h-full" />
        </ErrorBoundary>
      </div>

      {/* Command Buttons - shown when task is not running - fixed height */}
      {!isTaskRunning && availableCommands.length > 0 && (
        <div className="border-t border-border p-4 flex-shrink-0">
          <CommandButtons
            commands={availableCommands as Command[]}
            onCommandClick={handleCommandClick}
            disabled={sendMessage.isPending}
          />
        </div>
      )}

      {/* Chat Input Component - always visible at bottom - fixed height */}
      <div className="border-t border-border flex-shrink-0">
        <ErrorBoundary componentName="Chat Input">
          <div data-testid="task-chat-input" className="p-4">
            <div className="space-y-4">
              {lastIterationHasError && (
                <div className="px-4 py-3 bg-status-warning/10 border border-status-warning/20 rounded-lg">
                  <div className="flex items-start gap-3">
                    <div className="flex-1">
                      <p className="text-sm text-status-warning font-medium">
                        Previous iteration failed
                      </p>
                      <p className="text-sm text-status-warning mt-1">
                        You can choose to start fresh without context from the previous
                        conversation.
                      </p>
                    </div>
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={resumeConversation}
                        onChange={(e) => setResumeConversation(e.target.checked)}
                        className="rounded border-status-warning/30 text-status-warning focus:ring-status-warning/50"
                      />
                      <span className="text-sm text-status-warning whitespace-nowrap">
                        Resume conversation
                      </span>
                    </label>
                  </div>
                </div>
              )}
              <ChatInput
                message={message}
                onMessageChange={setMessage}
                onSubmit={handleChatSubmit}
                disabled={isSendDisabled}
                placeholder={getPlaceholderText()}
                repositoryId={repositoryId}
                taskId={params.taskId}
                isTaskContext={true}
                buttonText={getButtonText()}
                running={isTaskRunning}
                loading={sendMessage.isPending}
                maxHeight="150px"
                enableFileUpload={true}
                files={uploadedFiles}
                onFilesChange={setUploadedFiles}
                selectors={
                  <>
                    <PopoverSelector
                      icon={modelSelector.icon}
                      value={modelSelector.value}
                      displayValue={modelSelector.displayValue}
                      tooltip={modelSelector.tooltip}
                      loading={modelSelector.loading}
                      disabled={modelSelector.disabled}
                    >
                      <SimpleListPopoverContent
                        items={modelSelector.items}
                        value={modelSelector.value}
                        onChange={modelSelector.onChange}
                      />
                    </PopoverSelector>

                    <PopoverSelector
                      icon={agentModeSelector.icon}
                      value={agentModeSelector.value}
                      displayValue={agentModeSelector.displayValue}
                      tooltip={agentModeSelector.tooltip}
                      loading={agentModeSelector.loading}
                      disabled={agentModeSelector.disabled}
                    >
                      <SimpleListPopoverContent
                        items={agentModeSelector.items}
                        value={agentModeSelector.value}
                        onChange={agentModeSelector.onChange}
                      />
                    </PopoverSelector>
                  </>
                }
              />
            </div>
          </div>
        </ErrorBoundary>
      </div>
    </div>
  )
}

export const ChatView = React.memo(ChatViewComponent)
