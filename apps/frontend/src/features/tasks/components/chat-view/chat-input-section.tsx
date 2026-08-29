import type { MessageContextInput, UiMessage } from '@moatless/api-client'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useAgentAvailability } from '@/features/agents/hooks/use-agent-availability'
import { useRepositorySkills } from '@/features/skills'
import {
  AskUserQuestionInput,
  ChatInput,
  ContextItems,
  FileAttachments,
  PlanApprovalInput,
  SkillChips,
} from '@/features/tasks/components/chat/chat-input'
import {
  DEFAULT_AGENT_TYPE,
  useAgentModeSelector,
  useModelSelector,
} from '@/features/tasks/components/chat/chat-input/selectors'
import {
  CommandAutocomplete,
  type Suggestion,
} from '@/features/tasks/components/chat/command-autocomplete'
import { FileAutocomplete } from '@/features/tasks/components/chat/file-autocomplete'
import { QueuedMessages } from '@/features/tasks/components/chat/queued-messages'
import { usePanelControls, useTaskId } from '@/features/tasks/contexts/task-providers'
import { useSandboxStatus } from '@/features/tasks/hooks/use-sandbox-status'
import { useTaskDraftMessage } from '@/features/tasks/hooks/use-task-draft-message'
import { useTaskInputs } from '@/features/tasks/hooks/use-task-inputs'
import { useTaskDetailQuery } from '@/features/tasks/hooks/use-task-queries'
import { useStopAgent } from '@/features/tasks/hooks/use-tasks'
import { useFeatureFlag } from '@/lib/hooks/use-feature-flag'
import { type ClaudeModel, DEFAULT_CLAUDE_MODEL } from '@/lib/types/claude-models'
import { getErrorMessage } from '@/lib/utils/error'
import { ChatInputToolbar } from './chat-input-toolbar'
import { useChatAutocomplete } from './use-chat-autocomplete'
import { useChatSend } from './use-chat-send'
import type { PendingUserInput } from './use-pending-user-input'

interface ChatInputSectionProps {
  contextItems?: MessageContextInput[]
  onContextItemsChange?: ((contexts: MessageContextInput[]) => void) | undefined
  /** Hide the agent edit/plan mode toggle in the chat input. */
  hideAgentModeSelector?: boolean
  /** Pending AskUserQuestion / ExitPlanMode derived from the message list. */
  pendingUserInput: PendingUserInput | null
  /** Queued user messages derived from the message list, shown above the input. */
  queuedMessages: UiMessage[]
}

// Helper function to determine placeholder text based on environment and task status
function getPlaceholderText(
  envStatus: string | undefined,
  isTaskRunning: boolean,
  status: string | undefined,
): string {
  if (envStatus === 'initializing') {
    return 'Task queued: setting up sandbox...'
  }
  if (envStatus === 'error') {
    return 'Sandbox setup failed. Check error details above.'
  }
  if (isTaskRunning) {
    return 'Task is running... Type your next request'
  }
  if (status === 'closed' || status === 'error') {
    return `Task is ${status}. Type a new request to continue`
  }
  return 'Type a new request... (use @ to reference files)'
}

/**
 * The chat input area: pending-input prompts, the textarea with autocomplete,
 * toolbar, and related dialogs.
 *
 * Owns the per-keystroke state (draft message, autocomplete, selectors) so
 * typing re-renders only this subtree — never the message history above it,
 * which lives in ChatContainer.
 */
export function ChatInputSection({
  contextItems = [],
  onContextItemsChange,
  hideAgentModeSelector = false,
  pendingUserInput,
  queuedMessages,
}: ChatInputSectionProps) {
  const taskId = useTaskId()
  const { navigatePreview } = usePanelControls()

  const { data: task } = useTaskDetailQuery(taskId)
  const status = task?.status

  // Poll sandbox status only while the task remains open (hook handles gating)
  const sandboxStatusQuery = useSandboxStatus(taskId, task)
  const sandboxStatus = sandboxStatusQuery.data
  const envStatus = sandboxStatus?.sandboxStatus

  // Agent running state: driven by sandbox status (updated via SSE events + backend gRPC)
  const isTaskRunning = sandboxStatus?.agentStatus === 'running'

  // Fetch repo-scoped skills for the task's repository (workspace +
  // linked plugin repos), narrowed by the workspace the task runs in so the
  // dropdown offers what its agent will actually have. Surfaces in the "/"
  // autocomplete dropdown.
  const { data: repoSkills = [], error: skillsError } = useRepositorySkills(
    task?.repositoryId,
    task?.workspaceId ?? undefined,
    {
      enabled: !!task?.repositoryId && !!taskId && !isTaskRunning && envStatus === 'ready',
    },
  )

  useEffect(() => {
    if (skillsError) {
      toast.error('Failed to load repository skills', {
        description: getErrorMessage(skillsError),
      })
    }
  }, [skillsError])

  // Skill suggestions for the "/" autocomplete dropdown.
  const suggestions: Suggestion[] = repoSkills.map((s) => ({ kind: 'skill' as const, ...s }))

  // Local state for chat values. The message draft is persisted per task in
  // localStorage so text survives reloads and switching tasks.
  const [message, setMessage] = useTaskDraftMessage(taskId)
  const [resumeConversation] = useState(true)
  const [startFresh, setStartFresh] = useState(false)

  // Check if model selector should be shown
  const isModelSelectorEnabled = useFeatureFlag('SHOW_MODEL_SELECTOR')

  // Task inputs hook with external context management (from props)
  const taskInputs = useTaskInputs({
    contextItems,
    onContextItemsChange,
  })

  // "@" file + "/" skill autocomplete composition over the chat textarea
  const {
    textareaRef,
    handleMessageChange,
    handleCombinedKeyDown,
    fileAutocomplete,
    commandAutocomplete,
  } = useChatAutocomplete({
    taskId,
    message,
    setMessage,
    suggestions,
    commandsEnabled: !isTaskRunning,
    onSelectSkill: taskInputs.handleAddSkill,
  })

  // State for controlled selectors with defaults
  const [selectedModel, setSelectedModel] = useState<ClaudeModel>(DEFAULT_CLAUDE_MODEL)
  const [selectedAgentMode, setSelectedAgentMode] = useState<'edit' | 'plan'>('edit')

  // Initialize selectors with controlled state
  // Resolve the task's agent from the catalog so the model list + default are
  // scoped to whatever agent actually runs this task (e.g. codex, fable) rather
  // than the hard-coded Claude fallback.
  const { selectedAgent, availableModel } = useAgentAvailability({
    agentType: task?.agentType ?? DEFAULT_AGENT_TYPE,
    model: selectedModel,
  })

  const modelValue = isModelSelectorEnabled ? availableModel : DEFAULT_CLAUDE_MODEL
  const modelOnChange = isModelSelectorEnabled
    ? setSelectedModel
    : () => {
        /* no-op: model selector disabled */
      }

  const modelSelector = useModelSelector<ClaudeModel>(
    modelValue,
    modelOnChange,
    selectedAgent?.models,
    selectedAgent?.defaultModel ?? DEFAULT_CLAUDE_MODEL,
  )
  const agentModeSelector = useAgentModeSelector(selectedAgentMode, setSelectedAgentMode)

  // Stop agent mutation
  const stopAgent = useStopAgent(taskId)

  // Message-send orchestration (free text, pending-tool responses)
  const {
    sendMessage,
    handleChatSubmit,
    handleQuestionSubmit,
    handlePlanApprove,
    handlePlanReject,
  } = useChatSend({
    taskId,
    message,
    setMessage,
    taskInputs,
    selectedModel: availableModel,
    selectedAgentMode,
    setSelectedAgentMode,
    isModelSelectorEnabled,
    startFresh,
    setStartFresh,
    resumeConversation,
    pendingUserInput,
  })

  // Compute placeholder text using extracted helper
  const placeholderText = getPlaceholderText(envStatus, isTaskRunning, status)

  // Determine if we can submit based on conditions
  const canSubmit =
    !sendMessage.isPending && (!!message.trim() || taskInputs.contextItems.length > 0)

  return (
    <>
      {/* Queued messages stack above chat input (view-only) */}
      <QueuedMessages messages={queuedMessages} />

      {/* Conditionally render input based on pending user input */}
      {pendingUserInput?.type === 'AskUserQuestion' ? (
        <AskUserQuestionInput
          questions={pendingUserInput.questions}
          onSubmit={handleQuestionSubmit}
          isPending={sendMessage.isPending}
          error={sendMessage.error as Error | null}
        />
      ) : pendingUserInput?.type === 'ExitPlanMode' ? (
        <PlanApprovalInput
          plan={pendingUserInput.plan}
          onApprove={handlePlanApprove}
          onReject={handlePlanReject}
          isPending={sendMessage.isPending}
          error={sendMessage.error as Error | null}
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
          onKeyDownCapture={handleCombinedKeyDown}
          onPaste={taskInputs.handlePaste}
          positioned={true}
          autocompleteSlot={
            commandAutocomplete.isOpen ? (
              <CommandAutocomplete
                commands={commandAutocomplete.filteredCommands}
                selectedIndex={commandAutocomplete.selectedIndex}
                onSelect={commandAutocomplete.selectCommand}
                position="above"
              />
            ) : (
              fileAutocomplete.isOpen && (
                <FileAutocomplete
                  results={fileAutocomplete.results}
                  selectedIndex={fileAutocomplete.selectedIndex}
                  onSelect={fileAutocomplete.selectFile}
                  isLoading={fileAutocomplete.isLoading}
                  error={fileAutocomplete.error}
                  position="above"
                />
              )
            )
          }
        >
          <SkillChips skills={taskInputs.selectedSkills} onRemove={taskInputs.handleRemoveSkill} />

          {taskInputs.contextItems.length > 0 && (
            <ContextItems
              contexts={taskInputs.contextItems}
              onRemove={taskInputs.handleRemoveContext}
              onPreviewUrlClick={navigatePreview}
            />
          )}

          {taskInputs.uploadedFiles.length > 0 && (
            <FileAttachments
              files={taskInputs.uploadedFiles}
              onRemove={taskInputs.handleRemoveFile}
            />
          )}

          <ChatInputToolbar
            modelSelector={modelSelector}
            agentModeSelector={agentModeSelector}
            isModelSelectorEnabled={isModelSelectorEnabled}
            hideAgentModeSelector={hideAgentModeSelector}
            startFresh={startFresh}
            onToggleStartFresh={() => setStartFresh((v) => !v)}
            taskInputs={taskInputs}
            isTaskRunning={isTaskRunning}
            onStop={() => stopAgent.mutate()}
            isStopPending={stopAgent.isPending}
            canSubmit={canSubmit}
            isSendPending={sendMessage.isPending}
          />
        </ChatInput>
      )}
    </>
  )
}
