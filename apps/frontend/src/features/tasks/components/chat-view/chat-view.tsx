import type { ClaudeModel, Command, UploadFileResponse } from '@moatless/types'
import { useParams } from '@tanstack/react-router'
import React, { useCallback, useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useTaskData, useWorkspaceStatus } from '@/features/tasks/contexts/task-providers'
import {
  useChatMessage,
  useChatModel,
  useChatResumeConversation,
} from '@/features/tasks/contexts/task-ui-provider'
import { useSendMessage, useStopAgent, useTaskCommands } from '@/features/tasks/hooks/use-tasks'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { getErrorMessage } from '@/lib/utils/error'
import { ChatInput } from '../chat-input/chat-input'
import { GenericSelector, useModelSelector } from '../chat-input/selectors'
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

  // Chat UI state from TaskUI context
  const { message, setMessage, clearMessage } = useChatMessage()
  const { selectedModel, setSelectedModel } = useChatModel()
  const { resumeConversation, setResumeConversation } = useChatResumeConversation()

  // Local component state for file uploads
  const [uploadedFiles, setUploadedFiles] = useState<UploadFileResponse[]>([])

  // Model selector
  const modelSelector = useModelSelector(selectedModel)

  // Stop agent mutation
  const stopAgent = useStopAgent(params.taskId)

  // Update context when model selector changes (isolated per task, no localStorage sync)
  useEffect(() => {
    if (modelSelector.value && modelSelector.value !== selectedModel) {
      setSelectedModel(modelSelector.value as ClaudeModel)
    }
  }, [modelSelector.value, selectedModel, setSelectedModel])

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

  // Handle sending message - combines task message sending and chat input logic
  const handleChatSubmit = useCallback(() => {
    // If running, stop the agent instead of sending message
    if (status === 'running') {
      stopAgent.mutate()
      return
    }

    if (!message.trim()) {
      return
    }

    // Send message with file paths
    if (!sendMessage.isPending) {
      const filePaths = uploadedFiles.map((f) => f.absolutePath)

      // Create a stable callback that clears both message and files
      const clearMessageAndFiles = () => {
        clearMessage()
        setUploadedFiles([])
      }

      sendMessage.mutate(
        {
          message: message.trim(),
          resume: resumeConversation,
          model: modelSelector.value as ClaudeModel,
          filePaths,
        },
        {
          onSuccess: () => {
            clearMessageAndFiles()
          },
        },
      )
    }
  }, [
    status,
    stopAgent,
    message,
    sendMessage,
    uploadedFiles,
    clearMessage,
    resumeConversation,
    modelSelector.value,
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

  // Helper functions for ChatInput
  const getButtonText = () => {
    if (status === 'running') {
      return 'Running'
    }
    if (status === 'completed' || status === 'failed') {
      return 'Task Completed'
    }
    return sendMessage.isPending ? 'Sending...' : 'Send'
  }

  const getPlaceholderText = () => {
    if (status === 'running') {
      return 'Task is running... Type your next request'
    }
    if (status === 'completed' || status === 'failed') {
      return `Task is ${status}. Type a new request to continue`
    }
    return 'Type a new request... (use @ to reference files)'
  }

  // Check if last iteration has an error
  const lastIterationHasError =
    lastIteration && (lastIteration.status === 'error' || lastIteration.status === 'failed')

  const isTaskRunning = status === 'running'

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
                selectors={<GenericSelector {...modelSelector} />}
              />
            </div>
          </div>
        </ErrorBoundary>
      </div>
    </div>
  )
}

export const ChatView = React.memo(ChatViewComponent)
