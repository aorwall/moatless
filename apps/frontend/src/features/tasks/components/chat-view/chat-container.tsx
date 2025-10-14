import type { ClaudeModel, Message, MessageContext, UploadFileResponse } from '@moatless/types'
import { CLAUDE_MODELS } from '@moatless/types/constants'
import { useCallback, useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useFileSearch } from '@/features/file-viewer/api/use-file-search'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { useSendMessage, useStopAgent, useTaskMessages } from '@/features/tasks/hooks/use-tasks'
import { useWorkspaceStatus } from '@/features/tasks/hooks/use-workspace-status'
import { useImagePaste } from '@/features/uploads/hooks/use-image-paste'
import { useUploadFile } from '@/features/uploads/hooks/use-upload'
import {
  ChatInput,
  ContextItems,
  FileAttachments,
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
import { FileAutocomplete, useFileAutocomplete } from '@/lib/components/chat/file-autocomplete'
import { MessageHistory } from '@/lib/components/chat/message-history'
import { getErrorMessage } from '@/lib/utils/error'

interface ChatContainerProps {
  className?: string
  contextItems?: MessageContext[]
  onContextItemsChange?: (contexts: MessageContext[]) => void
}

function ChatContainer({ className, contextItems = [], onContextItemsChange }: ChatContainerProps) {
  const taskId = useTaskId()

  const { data: task } = useTaskDetailQuery(taskId)
  const status = task?.status
  const iterations = task?.iterations ?? []
  const isTaskRunning = status === 'running'

  // Poll workspace status only while the task remains open (hook handles gating)
  const workspaceStatusQuery = useWorkspaceStatus(taskId, task)
  const workspaceStatus = workspaceStatusQuery.data

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
      toast.error('Failed to upload file', {
        description: getErrorMessage(error),
      })
    },
  })

  // File search mutation (task-specific)
  const fileSearchMutation = useFileSearch(taskId)

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
    results:
      fileSearchMutation.data?.files.map((file) => ({
        path: file.path,
        absolutePath: file.path, // Use path as absolutePath for file search
        score: file.matches / 100, // Convert matches count to a score
      })) || [],
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
    // Clear contexts
    if (onContextItemsChange) {
      onContextItemsChange([])
    }
    // Reset selectors to new task's values or defaults
    const newModel = lastIterationModel || CLAUDE_MODELS.SONNET_4
    const newAgentMode = lastIterationAgentMode || 'edit'
    setSelectedModel(newModel)
    setSelectedAgentMode(newAgentMode)
  }, [taskId, lastIterationModel, lastIterationAgentMode, onContextItemsChange])

  // Stop agent mutation
  const stopAgent = useStopAgent(taskId)

  // Fetch messages for this task
  const {
    data: messages,
    isLoading: isLoadingMessages,
    error: messagesError,
  } = useTaskMessages(taskId, {
    enabled: !!taskId,
  }) as { data: Message[] | undefined; isLoading: boolean; error: unknown }

  // Task message sending hook
  const sendMessage = useSendMessage(taskId, {
    onError: (error) => {
      toast.error('Failed to send message', {
        description: getErrorMessage(error),
      })
    },
  })

  // Handler to remove context item
  const handleRemoveContext = useCallback(
    (index: number) => {
      if (onContextItemsChange) {
        const newContexts = contextItems.filter((_, i) => i !== index)
        onContextItemsChange(newContexts)
      }
    },
    [contextItems, onContextItemsChange],
  )

  // Handle sending message
  const handleChatSubmit = useCallback(() => {
    // If running, stop the agent instead of sending message
    if (isTaskRunning) {
      stopAgent.mutate()
      return
    }

    // Send message with file IDs and contexts
    if (!sendMessage.isPending && message.trim()) {
      const fileIds = uploadedFiles.map((f) => f.fileId)

      sendMessage.mutate(
        {
          message: message.trim(),
          resume: resumeConversation,
          model: selectedModel,
          fileIds,
          contexts: contextItems.length > 0 ? contextItems : undefined,
          agentMode: selectedAgentMode,
        },
        {
          onSuccess: () => {
            // Clear state after successful send
            setMessage('')
            setUploadedFiles([])
            // Clear contexts
            if (onContextItemsChange) {
              onContextItemsChange([])
            }
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
    contextItems,
    resumeConversation,
    selectedModel,
    selectedAgentMode,
    onContextItemsChange,
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
    if (status === 'closed' || status === 'error') {
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
    if (status === 'closed' || status === 'error') {
      return `Task is ${status}. Type a new request to continue`
    }
    return 'Type a new request... (use @ to reference files)'
  }

  // Determine if we can submit based on conditions
  const envStatus = workspaceStatus?.environmentStatus
  const isWorkspaceSetup = envStatus === 'initializing'
  const isWorkspaceError = envStatus === 'error'
  const canSubmit =
    !(sendMessage.isPending || isWorkspaceSetup || isWorkspaceError) &&
    (isTaskRunning || !!message.trim())

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

  // Handle image paste
  const handlePaste = useImagePaste(uploadFile)

  return (
    <ChatView className={className}>
      <MessageHistory
        messages={messages}
        isLoading={isLoadingMessages}
        error={messagesError as Error | null}
        isRunning={isTaskRunning}
      />

      <ChatInput
        message={message}
        onMessageChange={handleMessageChange}
        onSubmit={handleChatSubmit}
        loading={sendMessage.isPending}
        placeholder={getPlaceholderText()}
        canSubmit={canSubmit}
        textareaRef={textareaRef}
        onKeyDownCapture={handleAutocompleteKeyDown}
        onPaste={handlePaste}
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
        {contextItems.length > 0 && (
          <ContextItems contexts={contextItems} onRemove={handleRemoveContext} />
        )}

        {uploadedFiles.length > 0 && (
          <FileAttachments
            files={uploadedFiles}
            onRemove={(fileId) =>
              setUploadedFiles((prev) => prev.filter((f) => f.fileId !== fileId))
            }
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
              disabled={!canSubmit}
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
