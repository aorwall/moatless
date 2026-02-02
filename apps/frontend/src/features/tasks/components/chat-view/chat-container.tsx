import type {
  AssistantMessage,
  ClaudeModel,
  Command,
  Message,
  MessageContext,
  Question,
  UserMessage,
} from '@moatless/types'
import { DEFAULT_CLAUDE_MODEL } from '@moatless/types/constants'
import { AlertCircle, Loader2 } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { FigmaImportDialog } from '@/features/figma/components/figma-import-dialog'
import { SettingsDialog, useFigmaConfig } from '@/features/settings'
import { useTaskId } from '@/features/tasks/contexts/task-providers'
import { useFileSearch } from '@/features/tasks/hooks/use-file-search'
import { useSubmitUserInput } from '@/features/tasks/hooks/use-submit-user-input'
import { useTaskInputs } from '@/features/tasks/hooks/use-task-inputs'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import {
  useDeleteMessage,
  useSendMessage,
  useStopAgent,
  useTaskCommands,
  useTaskMessages,
} from '@/features/tasks/hooks/use-tasks'
import { useWorkspaceRetry, useWorkspaceStatus } from '@/features/tasks/hooks/use-workspace-status'
import {
  AskUserQuestionInput,
  ChatInput,
  ContextItems,
  FileAttachments,
  PlanApprovalInput,
  Selectors,
  SendButton,
  StopButton,
} from '@/lib/components/chat/chat-input'
import {
  AttachmentPopoverContent,
  PopoverSelector,
  SimpleListPopoverContent,
  useAgentModeSelector,
  useModelSelector,
} from '@/lib/components/chat/chat-input/selectors'
import { ChatView } from '@/lib/components/chat/chat-view'
import { FileAutocomplete, useFileAutocomplete } from '@/lib/components/chat/file-autocomplete'
import { MessageHistory } from '@/lib/components/chat/message-history'
import { QueuedMessages } from '@/lib/components/chat/queued-messages'
import { Button } from '@/lib/components/ui/button'
import { useFeatureFlag } from '@/lib/hooks/use-feature-flag'
import { getErrorMessage } from '@/lib/utils/error'
import { CommandButtons } from './command-buttons'

interface ChatContainerProps {
  className?: string
  contextItems?: MessageContext[]
  onContextItemsChange?: (contexts: MessageContext[]) => void
  onPreviewUrlClick?: (path: string) => void
  onCommandExecutorReady?: (executor: (commandName: string) => void) => void
}

// Helper function to determine placeholder text based on environment and task status
function getPlaceholderText(
  envStatus: string | undefined,
  isTaskRunning: boolean,
  status: string | undefined,
): string {
  if (envStatus === 'initializing') {
    return 'Task queued: setting up workspace...'
  }
  if (envStatus === 'error') {
    return 'Workspace setup failed. Check error details above.'
  }
  if (isTaskRunning) {
    return 'Task is running... Type your next request'
  }
  if (status === 'closed' || status === 'error') {
    return `Task is ${status}. Type a new request to continue`
  }
  return 'Type a new request... (use @ to reference files)'
}

// Helper function to determine the model value for a task
function getModelForTask(
  isModelSelectorEnabled: boolean,
  lastTurnModel: ClaudeModel | undefined,
): ClaudeModel {
  if (!isModelSelectorEnabled) {
    return DEFAULT_CLAUDE_MODEL
  }
  return lastTurnModel || DEFAULT_CLAUDE_MODEL
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Chat container with multiple interaction modes
function ChatContainer({
  className,
  contextItems = [],
  onContextItemsChange,
  onPreviewUrlClick,
  onCommandExecutorReady,
}: ChatContainerProps) {
  const taskId = useTaskId()

  const { data: task } = useTaskDetailQuery(taskId)
  const status = task?.status
  const isTaskRunning = status === 'running'

  // Poll workspace status only while the task remains open (hook handles gating)
  const workspaceStatusQuery = useWorkspaceStatus(taskId, task)
  const workspaceStatus = workspaceStatusQuery.data
  const envStatus = workspaceStatus?.environmentStatus

  // Retry workspace mutation for handling failed provisioning
  const retryWorkspaceMutation = useWorkspaceRetry(taskId)

  // Fetch available commands when task is not running
  const { data: commandsData } = useTaskCommands(taskId, {
    enabled: !!taskId && !isTaskRunning && envStatus === 'ready',
  })
  const availableCommands = commandsData?.commands ?? []

  // Local state for chat values
  const [message, setMessage] = useState('')
  const [resumeConversation] = useState(true)
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false)

  // Check Figma configuration status
  const { data: figmaConfig } = useFigmaConfig()

  // Check if model selector should be shown
  const isModelSelectorEnabled = useFeatureFlag('SHOW_MODEL_SELECTOR')

  // Task inputs hook with external context management (from props)
  const taskInputs = useTaskInputs({
    enableContextItems: true,
    enableFigma: true,
    contextItems,
    onContextItemsChange,
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

  // State for controlled selectors with defaults
  const [selectedModel, setSelectedModel] = useState<ClaudeModel>(
    getModelForTask(isModelSelectorEnabled, undefined),
  )
  const [selectedAgentMode, setSelectedAgentMode] = useState<'edit' | 'plan'>('edit')

  // Initialize selectors with controlled state
  const modelValue = isModelSelectorEnabled ? selectedModel : DEFAULT_CLAUDE_MODEL
  const modelOnChange = isModelSelectorEnabled
    ? setSelectedModel
    : () => {
        /* no-op: model selector disabled */
      }
  const modelSelector = useModelSelector(modelValue, modelOnChange)
  const agentModeSelector = useAgentModeSelector(selectedAgentMode, setSelectedAgentMode)

  // Clear state when task changes
  // biome-ignore lint/correctness/useExhaustiveDependencies: taskId is from params and we want to reset state when it changes
  useEffect(() => {
    setMessage('')
    taskInputs.clearAll()
    // Reset selectors to defaults
    setSelectedModel(getModelForTask(isModelSelectorEnabled, undefined))
    setSelectedAgentMode('edit')
  }, [taskId, isModelSelectorEnabled])

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

  // Derive queued messages from the messages list
  const queuedMessages = useMemo(
    () =>
      (messages ?? []).filter(
        (m): m is UserMessage => m.role === 'user' && (m as UserMessage).isQueued === true,
      ),
    [messages],
  )

  // Delete queued message mutation
  const deleteMessage = useDeleteMessage(taskId)

  // User input submission hook (for AskUserQuestion and ExitPlanMode)
  const submitUserInput = useSubmitUserInput(taskId)

  // Detect pending user input from the last message
  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Complex nested logic to extract pending tool info
  const pendingUserInput = useMemo(() => {
    if (!messages || messages.length === 0) {
      return null
    }

    const lastMessage = messages[messages.length - 1]
    if (lastMessage.role !== 'assistant') {
      return null
    }

    const assistantMessage = lastMessage as AssistantMessage
    if (!assistantMessage.hasPendingUserInput) {
      return null
    }

    // Find the pending tool call (last tool_use without a result)
    if (!Array.isArray(assistantMessage.content)) {
      return null
    }

    for (let i = assistantMessage.content.length - 1; i >= 0; i--) {
      const content = assistantMessage.content[i]
      if (content.type === 'tool_use' && !content.tool_result) {
        if (content.tool_name === 'AskUserQuestion' && content.tool_input) {
          const toolInput = content.tool_input as { questions?: Question[] }
          if (toolInput.questions && Array.isArray(toolInput.questions)) {
            return {
              type: 'AskUserQuestion' as const,
              questions: toolInput.questions,
            }
          }
        }
        if (content.tool_name === 'ExitPlanMode' && content.tool_input) {
          const toolInput = content.tool_input as { plan?: string }
          return {
            type: 'ExitPlanMode' as const,
            plan: toolInput.plan,
            toolUseId: content.tool_use_id,
          }
        }
      }
    }

    return null
  }, [messages])

  // Handlers for user input submission
  const handleQuestionSubmit = useCallback(
    (answers: Record<string, string>) => {
      submitUserInput.mutate({
        toolName: 'AskUserQuestion',
        response: { answers },
      })
    },
    [submitUserInput],
  )

  const handlePlanApprove = useCallback(() => {
    submitUserInput.mutate({
      toolName: 'ExitPlanMode',
      response: { approved: true },
    })
    // Auto-switch to edit mode after approving plan
    setSelectedAgentMode('edit')
  }, [submitUserInput])

  const handlePlanReject = useCallback(
    (feedback?: string) => {
      submitUserInput.mutate({
        toolName: 'ExitPlanMode',
        response: { approved: false, feedback },
      })
    },
    [submitUserInput],
  )

  // Handle sending message
  const handleChatSubmit = useCallback(() => {
    const trimmedMessage = message.trim()
    if (sendMessage.isPending || !trimmedMessage) {
      return
    }

    const messageData = {
      message: trimmedMessage,
      resume: resumeConversation,
      model: isModelSelectorEnabled ? selectedModel : DEFAULT_CLAUDE_MODEL,
      fileIds: taskInputs.getFileIds(),
      contexts: taskInputs.contextItems.length > 0 ? taskInputs.contextItems : undefined,
      agentMode: selectedAgentMode,
    }

    sendMessage.mutate(messageData, {
      onSuccess: () => {
        setMessage('')
        taskInputs.clearAll()
      },
    })
  }, [
    sendMessage,
    message,
    resumeConversation,
    selectedModel,
    selectedAgentMode,
    taskInputs,
    isModelSelectorEnabled,
  ])

  // Handle command button click
  const handleCommandClick = useCallback(
    (command: Command) => {
      const commandData = {
        message: command.prompt,
        commandName: command.name,
        resume: command.willNotResume !== true,
        model: isModelSelectorEnabled ? selectedModel : DEFAULT_CLAUDE_MODEL,
        agentMode: command.agentMode || selectedAgentMode,
        fileIds: taskInputs.getFileIds(),
        contexts: taskInputs.contextItems.length > 0 ? taskInputs.contextItems : undefined,
      }

      sendMessage.mutate(commandData, {
        onSuccess: () => {
          taskInputs.clearAll()
        },
      })
    },
    [sendMessage, isModelSelectorEnabled, selectedModel, selectedAgentMode, taskInputs],
  )

  // Execute a command by name — used by external callers (e.g. preview troubleshoot button)
  const executeCommandByName = useCallback(
    (commandName: string) => {
      const command = availableCommands.find((c) => c.name === commandName) as Command | undefined
      if (!command) {
        return
      }

      const commandData = {
        message: command.prompt,
        commandName: command.name,
        resume: command.willNotResume !== true,
        model: isModelSelectorEnabled ? selectedModel : DEFAULT_CLAUDE_MODEL,
        agentMode: command.agentMode || selectedAgentMode,
      }
      sendMessage.mutate(commandData)
    },
    [availableCommands, sendMessage, isModelSelectorEnabled, selectedModel, selectedAgentMode],
  )

  // Expose command executor to parent
  useEffect(() => {
    onCommandExecutorReady?.(executeCommandByName)
  }, [onCommandExecutorReady, executeCommandByName])

  // Compute placeholder text using extracted helper
  const placeholderText = getPlaceholderText(envStatus, isTaskRunning, status)

  // Determine if we can submit based on conditions
  const canSubmit =
    !sendMessage.isPending && (!!message.trim() || taskInputs.contextItems.length > 0)

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

  // Handle Figma import click - check config and open appropriate dialog
  const handleFigmaImportClick = useCallback(() => {
    if (figmaConfig?.connected) {
      taskInputs.setFigmaDialogOpen(true)
    } else {
      setSettingsDialogOpen(true)
    }
  }, [figmaConfig?.connected, taskInputs])

  return (
    <ChatView className={className}>
      <MessageHistory
        messages={messages}
        isLoading={isLoadingMessages}
        error={messagesError as Error | null}
        isRunning={isTaskRunning}
        environmentStatus={envStatus}
        environmentError={workspaceStatus?.environmentError}
        onPreviewUrlClick={onPreviewUrlClick}
      />

      {/* Error banner for failed tasks */}
      {task?.status === 'error' && (
        <div className="mx-4 mb-2 px-4 py-3 bg-destructive/10 border border-destructive/20 rounded-lg">
          <div className="flex items-start gap-2">
            <AlertCircle className="h-4 w-4 text-destructive mt-0.5 shrink-0" />
            <div className="min-w-0 flex-1">
              <p className="text-sm text-destructive font-medium">Agent Error</p>
              <p className="text-sm text-muted-foreground whitespace-pre-wrap">
                Task failed. Check the message history for details.
              </p>
            </div>
          </div>
        </div>
      )}

      {/* Workspace error banner with retry button */}
      {envStatus === 'error' && workspaceStatus?.environmentError && (
        <div className="mx-4 mb-2 px-4 py-3 bg-destructive/10 border border-destructive/20 rounded-lg flex items-center justify-between gap-4">
          <div className="min-w-0 flex-1">
            <p className="text-sm text-destructive font-medium">Workspace setup failed</p>
            <p className="text-sm text-muted-foreground truncate">
              {workspaceStatus.environmentError}
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={() => retryWorkspaceMutation.mutate()}
            disabled={retryWorkspaceMutation.isPending}
          >
            {retryWorkspaceMutation.isPending ? (
              <>
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                Retrying...
              </>
            ) : (
              'Retry'
            )}
          </Button>
        </div>
      )}

      {/* Command buttons — horizontal bar above chat input */}
      {!isTaskRunning && availableCommands.length > 0 && !pendingUserInput && (
        <CommandButtons
          commands={availableCommands as Command[]}
          onCommandClick={handleCommandClick}
          disabled={sendMessage.isPending}
        />
      )}

      {/* Queued messages stack above chat input */}
      <QueuedMessages
        messages={queuedMessages}
        onRemove={(messageId) => deleteMessage.mutate(messageId)}
      />

      {/* Conditionally render input based on pending user input */}
      {pendingUserInput?.type === 'AskUserQuestion' ? (
        <AskUserQuestionInput
          questions={pendingUserInput.questions}
          onSubmit={handleQuestionSubmit}
          isPending={submitUserInput.isPending}
          error={submitUserInput.error as Error | null}
        />
      ) : pendingUserInput?.type === 'ExitPlanMode' ? (
        <PlanApprovalInput
          plan={pendingUserInput.plan}
          onApprove={handlePlanApprove}
          onReject={handlePlanReject}
          isPending={submitUserInput.isPending}
          error={submitUserInput.error as Error | null}
        />
      ) : (
        <ChatInput
          message={message}
          onMessageChange={handleMessageChange}
          onSubmit={handleChatSubmit}
          loading={sendMessage.isPending}
          placeholder={placeholderText}
          canSubmit={canSubmit}
          textareaRef={textareaRef}
          onKeyDownCapture={handleAutocompleteKeyDown}
          onPaste={taskInputs.handlePaste}
          positioned={true}
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
          {(taskInputs.contextItems.length > 0 || taskInputs.figmaImportPending) && (
            <ContextItems
              contexts={taskInputs.contextItems}
              onRemove={taskInputs.handleRemoveContext}
              onPreviewUrlClick={onPreviewUrlClick}
              isLoading={taskInputs.figmaImportPending}
            />
          )}

          {taskInputs.uploadedFiles.length > 0 && (
            <FileAttachments
              files={taskInputs.uploadedFiles}
              onRemove={taskInputs.handleRemoveFile}
            />
          )}

          <div className="flex items-center justify-between gap-2">
            <Selectors>
              {isModelSelectorEnabled && (
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
              )}

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
              <PopoverSelector
                icon={taskInputs.attachmentSelector.icon}
                value={taskInputs.attachmentSelector.value}
                displayValue={taskInputs.attachmentSelector.displayValue}
                tooltip={taskInputs.attachmentSelector.tooltip}
                loading={taskInputs.attachmentSelector.loading}
                disabled={taskInputs.attachmentSelector.disabled}
              >
                <AttachmentPopoverContent
                  onFileSelect={(file) => taskInputs.uploadFile.mutate(file)}
                  onFigmaImport={handleFigmaImportClick}
                  disabled={taskInputs.uploadFile.isPending}
                />
              </PopoverSelector>
              {isTaskRunning && (
                <StopButton onClick={() => stopAgent.mutate()} disabled={stopAgent.isPending} />
              )}
              <SendButton disabled={!canSubmit} loading={sendMessage.isPending} />
            </div>
          </div>
        </ChatInput>
      )}

      <FigmaImportDialog
        open={taskInputs.figmaDialogOpen}
        onOpenChange={taskInputs.setFigmaDialogOpen}
        importMutation={taskInputs.figmaUrlImport}
      />

      <SettingsDialog
        open={settingsDialogOpen}
        onOpenChange={setSettingsDialogOpen}
        defaultSection="figma"
      />
    </ChatView>
  )
}

export { ChatContainer }
