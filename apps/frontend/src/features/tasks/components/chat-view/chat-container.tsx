import type { ClaudeModel, Command, Message, UploadFileResponse } from '@moatless/types'
import { CLAUDE_MODELS } from '@moatless/types/constants'
import { useParams } from '@tanstack/react-router'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useIsAgentRunning, useTaskData } from '@/features/tasks/contexts/task-providers'
import {
  useSendMessage,
  useStopAgent,
  useTaskCommands,
  useTaskMessages,
} from '@/features/tasks/hooks/use-tasks'
import { useUploadFile } from '@/features/uploads/hooks/use-upload'
import {
  ChatInput,
  Files,
  Selectors,
  SendButton,
  UploadButton,
} from '@/lib/components/chat/chat-input'
import {
  PopoverSelector,
  SimpleListPopoverContent,
  useAgentModeSelector,
  useModelSelector,
} from '@/lib/components/chat/chat-input/selectors'
import { ChatView } from '@/lib/components/chat/chat-view'
import { MessageHistory } from '@/lib/components/chat/message-history'
import { getErrorMessage } from '@/lib/utils/error'

interface ChatContainerProps {
  className?: string
}

function ChatContainer({ className }: ChatContainerProps) {
  const params = useParams({ from: '/tasks/$taskId' })

  // Use specialized hooks from tri-context pattern
  const { status, iterations } = useTaskData()
  const isTaskRunning = useIsAgentRunning()

  // Local state for chat values
  const [message, setMessage] = useState('')
  const [uploadedFiles, setUploadedFiles] = useState<UploadFileResponse[]>([])
  const [resumeConversation, setResumeConversation] = useState(true)

  // File upload hook
  const uploadFile = useUploadFile({
    onSuccess: (data) => {
      setUploadedFiles((prev) => [...prev, data])
    },
    onError: (error) => {
      console.error('File upload failed:', error)
      toast.error('Failed to upload file', {
        description: getErrorMessage(error),
      })
    },
  })

  // Derive initial values from last iteration
  const lastIteration =
    iterations && iterations.length > 0 ? iterations[iterations.length - 1] : null
  const lastIterationModel = lastIteration?.model as ClaudeModel
  const lastIterationAgentMode = lastIteration?.agentMode as 'edit' | 'plan'

  // State for controlled selectors
  const [selectedModel, setSelectedModel] = useState<ClaudeModel>(
    lastIterationModel || CLAUDE_MODELS.SONNET_4,
  )
  const [selectedAgentMode, setSelectedAgentMode] = useState<'edit' | 'plan'>(
    lastIterationAgentMode || 'edit',
  )

  // Initialize selectors with controlled state
  const modelSelector = useModelSelector(selectedModel, setSelectedModel)
  const agentModeSelector = useAgentModeSelector(selectedAgentMode, setSelectedAgentMode)

  // Clear state when task changes
  // biome-ignore lint/correctness/useExhaustiveDependencies: taskId is from params and we want to reset state when it changes
  useEffect(() => {
    setMessage('')
    setUploadedFiles([])
    // Reset selectors to new task's values or defaults
    const newModel = lastIterationModel || CLAUDE_MODELS.SONNET_4
    const newAgentMode = lastIterationAgentMode || 'edit'
    setSelectedModel(newModel)
    setSelectedAgentMode(newAgentMode)
  }, [params.taskId, lastIterationModel, lastIterationAgentMode])

  // Stop agent mutation
  const stopAgent = useStopAgent(params.taskId)

  // Fetch messages for this task
  const {
    data: messages,
    isLoading: isLoadingMessages,
    error: messagesError,
  } = useTaskMessages(params.taskId, {
    enabled: !!params.taskId,
  }) as { data: Message[] | undefined; isLoading: boolean; error: unknown }

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
          model: selectedModel,
          filePaths,
          agentMode: selectedAgentMode,
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
    selectedModel,
    selectedAgentMode,
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
      <ChatView className="flex-1">
        <MessageHistory
          messages={messages}
          isLoading={isLoadingMessages}
          error={messagesError as Error | null}
          isRunning={isTaskRunning}
        />
        <ChatInput
          message={message}
          onMessageChange={setMessage}
          onSubmit={handleChatSubmit}
          loading={sendMessage.isPending}
          placeholder={getPlaceholderText()}
          className="max-h-[150px]"
        >
          {uploadedFiles.length > 0 && (
            <Files
              files={uploadedFiles}
              onRemove={(id) => setUploadedFiles((prev) => prev.filter((f) => f.id !== id))}
            />
          )}

          <div className="flex items-center justify-between gap-2">
            <Selectors>
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
            </Selectors>

            <div className="flex items-center gap-2">
              <UploadButton
                onFileSelect={(file) => uploadFile.mutate(file)}
                disabled={uploadFile.isPending}
              />
              <SendButton
                onSubmit={handleChatSubmit}
                disabled={isSendDisabled}
                loading={sendMessage.isPending}
                running={isTaskRunning}
                buttonText={getButtonText()}
              />
            </div>
          </div>
        </ChatInput>
      </ChatView>
    </div>
  )
}

export { ChatContainer as ChatView }
