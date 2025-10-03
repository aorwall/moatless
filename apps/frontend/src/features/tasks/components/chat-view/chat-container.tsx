import type { ClaudeModel, Message, UploadFileResponse } from '@moatless/types'
import { CLAUDE_MODELS } from '@moatless/types/constants'
import { useParams } from '@tanstack/react-router'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useFileSearch } from '@/features/file-viewer/api/use-file-search'
import { useIsAgentRunning, useTaskData } from '@/features/tasks/contexts/task-providers'
import { useSendMessage, useStopAgent, useTaskMessages } from '@/features/tasks/hooks/use-tasks'
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
import {
  ChatStatusBanner,
  type ChatStatusBannerProps,
} from '@/lib/components/chat/chat-status-banner'
import { ChatView } from '@/lib/components/chat/chat-view'
import { FileAutocomplete, useFileAutocomplete } from '@/lib/components/chat/file-autocomplete'
import { MessageHistory } from '@/lib/components/chat/message-history'
import { getErrorMessage } from '@/lib/utils/error'

interface ChatContainerProps {
  className?: string
}

function ChatContainer({ className }: ChatContainerProps) {
  const params = useParams({ from: '/tasks/$taskId' })

  // Use specialized hooks from tri-context pattern
  const { status, iterations, workspaceStatus } = useTaskData()
  const isTaskRunning = useIsAgentRunning()

  // Local state for chat values
  const [message, setMessage] = useState('')
  const [uploadedFiles, setUploadedFiles] = useState<UploadFileResponse[]>([])
  const [resumeConversation] = useState(true)

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

  // File search mutation (task-specific)
  const fileSearchMutation = useFileSearch(params.taskId)

  // File autocomplete hook (generic)
  const {
    inputRef: textareaRef,
    isOpen: isAutocompleteOpen,
    results: autocompleteResults,
    selectedIndex: autocompleteSelectedIndex,
    isLoading: isAutocompleteLoading,
    error: autocompleteError,
    handleInputChange,
    handleKeyDown: handleAutocompleteKeyDown,
    selectFile,
  } = useFileAutocomplete({
    value: message,
    onValueChange: setMessage,
    enabled: true,
    onSearch: fileSearchMutation.mutate,
    results: fileSearchMutation.data?.results || [],
    isLoading: fileSearchMutation.isPending,
    error: fileSearchMutation.error,
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

  // Helper functions for ChatInput
  const getButtonText = () => {
    const envStatus = workspaceStatus?.environmentStatus

    if (envStatus === 'initializing') {
      return 'Queued'
    }
    if (envStatus === 'error') {
      return 'Setup Failed'
    }
    if (isTaskRunning) {
      return 'Running'
    }
    if (status === 'completed' || status === 'failed') {
      return 'Task Completed'
    }
    return sendMessage.isPending ? 'Sending...' : 'Send'
  }

  const getPlaceholderText = () => {
    const envStatus = workspaceStatus?.environmentStatus

    if (envStatus === 'initializing') {
      return 'Task queued: setting up workspace...'
    }
    if (envStatus === 'error') {
      return 'Workspace setup failed. Check error details above.'
    }
    if (lastIteration?.status === 'created' && envStatus === 'ready') {
      return 'Task queued: starting agent...'
    }
    if (isTaskRunning) {
      return 'Task is running... Type your next request'
    }
    if (status === 'completed' || status === 'failed') {
      return `Task is ${status}. Type a new request to continue`
    }
    return 'Type a new request... (use @ to reference files)'
  }

  // Disable sending based on conditions
  const envStatus = workspaceStatus?.environmentStatus
  const isWorkspaceSetup = envStatus === 'initializing'
  const isWorkspaceError = envStatus === 'error'
  const isSendDisabled =
    sendMessage.isPending ||
    isWorkspaceSetup ||
    isWorkspaceError ||
    (!isTaskRunning && !message.trim())

  // Handle message changes with autocomplete
  const handleMessageChange = useCallback(
    (newMessage: string) => {
      setMessage(newMessage)
      // Update autocomplete state based on cursor position
      if (textareaRef.current) {
        const cursorPosition = textareaRef.current.selectionStart || 0
        handleInputChange(newMessage, cursorPosition)
      }
    },
    [handleInputChange, textareaRef],
  )

  // Derive status banner props from workspace and iteration state
  const statusBannerProps: ChatStatusBannerProps | null = useMemo(() => {
    const envStatus = workspaceStatus?.environmentStatus
    const envError = workspaceStatus?.environmentError

    // Task queued - workspace initializing
    if (envStatus === 'initializing') {
      return {
        variant: 'loading',
        message: 'Task queued',
        description: 'Setting up workspace: installing dependencies and configuring environment...',
      }
    }

    // Workspace setup failed
    if (envStatus === 'error') {
      return {
        variant: 'error',
        message: 'Workspace setup failed',
        description: envError || 'An error occurred during workspace setup',
      }
    }

    // Task queued - iteration waiting to start (workspace is ready but agent hasn't started)
    if (lastIteration?.status === 'created' && envStatus === 'ready') {
      return {
        variant: 'info',
        message: 'Task queued',
        description: 'Starting agent...',
      }
    }

    // Iteration failed
    if (lastIteration?.status === 'failed' && !isTaskRunning) {
      return {
        variant: 'error',
        message: 'Task failed',
        description: lastIteration.result || 'The task encountered an error',
      }
    }

    return null
  }, [workspaceStatus, lastIteration, isTaskRunning])

  return (
    <ChatView className={className}>
      <MessageHistory
        messages={messages}
        isLoading={isLoadingMessages}
        error={messagesError as Error | null}
        isRunning={isTaskRunning}
      />

      {statusBannerProps && <ChatStatusBanner {...statusBannerProps} />}

      <ChatInput
        message={message}
        onMessageChange={handleMessageChange}
        onSubmit={handleChatSubmit}
        loading={sendMessage.isPending}
        placeholder={getPlaceholderText()}
        textareaRef={textareaRef}
        onKeyDownCapture={handleAutocompleteKeyDown}
        autocompleteSlot={
          isAutocompleteOpen && (
            <FileAutocomplete
              results={autocompleteResults}
              selectedIndex={autocompleteSelectedIndex}
              onSelect={selectFile}
              isLoading={isAutocompleteLoading}
              error={autocompleteError}
              position="above"
            />
          )
        }
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
  )
}

export { ChatContainer }
