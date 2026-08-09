import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useAgentAvailability } from '@/features/agents'
import { useRepository } from '@/features/repositories'
import { SettingsDialog } from '@/features/settings'
import { useRepositorySkills } from '@/features/skills'
import {
  ChatInput,
  ContextItems,
  FileAttachments,
  Selectors,
  SendButton,
  SkillChips,
} from '@/features/tasks/components/chat/chat-input'
import {
  type AgentType,
  AttachmentPopoverContent,
  BranchPopoverContent,
  DEFAULT_AGENT_TYPE,
  PopoverSelector,
  SearchablePopoverContent,
  SimpleListPopoverContent,
  useAgentModeSelector,
  useAgentTypeSelector,
  useBranchSelector,
  useModelSelector,
  useWorkspaceSelector,
} from '@/features/tasks/components/chat/chat-input/selectors'
import {
  CommandAutocomplete,
  useCommandAutocomplete,
} from '@/features/tasks/components/chat/command-autocomplete'
import { useCodeTaskSubmit } from '@/features/tasks/components/create-task/use-code-task-submit'
import { useTaskInputs } from '@/features/tasks/hooks/use-task-inputs'
import { useWorkspace, useWorkspacesList } from '@/features/workspaces/hooks/use-workspaces'
import { useFeatureFlag } from '@/lib/hooks/use-feature-flag'
import { type ClaudeModel, DEFAULT_CLAUDE_MODEL, modelToApiValue } from '@/lib/types/claude-models'
import { getErrorMessage } from '@/lib/utils/error'

interface CreateCodeTaskProps {
  className?: string | undefined
  maxHeight?: string | undefined
  minHeight?: string | undefined
  showTitle?: boolean | undefined
  title?: string | undefined
  defaultMessage?: string | undefined
  model: ClaudeModel
  onModelChange: (model: ClaudeModel) => void
  agentMode: 'edit' | 'plan'
  onAgentModeChange: (mode: 'edit' | 'plan') => void
  agentType?: AgentType | undefined
  onAgentTypeChange?: ((type: AgentType) => void) | undefined
  workspaceId: string
  onWorkspaceChange: (workspaceId: string) => void
  onSuccess?: (() => void) | undefined
}

export function CreateCodeTask({
  className = '',
  maxHeight = '300px',
  minHeight,
  showTitle = false,
  title = 'What should we build today?',
  defaultMessage,
  model,
  onModelChange,
  agentMode,
  onAgentModeChange,
  agentType = DEFAULT_AGENT_TYPE,
  onAgentTypeChange,
  workspaceId,
  onWorkspaceChange,
  onSuccess,
}: CreateCodeTaskProps) {
  const { submit, isSubmitting } = useCodeTaskSubmit()
  const [message, setMessage] = useState(defaultMessage || '')
  const [branch, setBranch] = useState('')
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false)

  const { agents, selectedAgent, agentUnavailable, settingsSection, availableModel } =
    useAgentAvailability({ agentType, model })

  // Derive primary repo from workspace for branch/skills
  const { data: workspaceDetails } = useWorkspace(workspaceId, { enabled: !!workspaceId })
  const primaryRepoId = workspaceDetails?.repos.find((r) => r.isPrimary)?.repositoryId ?? undefined

  const { data: skills = [], error: skillsError } = useRepositorySkills(primaryRepoId)

  // Surface skill-fetch failures via a toast.
  useEffect(() => {
    if (skillsError) {
      toast.error('Failed to load repository skills', {
        description: getErrorMessage(skillsError),
      })
    }
  }, [skillsError])

  // Skill suggestions for the "/" dropdown.
  const suggestions = skills.map((s) => ({ kind: 'skill' as const, ...s }))

  const taskInputs = useTaskInputs()

  // Skill autocomplete hook
  const {
    inputRef: textareaRef,
    isOpen: isCommandAutocompleteOpen,
    filteredCommands,
    selectedIndex: commandSelectedIndex,
    handleInputChange: handleCommandInputChange,
    handleKeyDown: handleCommandKeyDown,
    selectCommand,
  } = useCommandAutocomplete({
    value: message,
    onValueChange: setMessage,
    onSelectSkill: taskInputs.handleAddSkill,
    suggestions,
    enabled: true,
  })

  const isModelSelectorEnabled = useFeatureFlag('SHOW_MODEL_SELECTOR')

  const { data: workspaces } = useWorkspacesList()
  const { data: repositoryDetails } = useRepository(primaryRepoId ?? '', {
    enabled: !!primaryRepoId,
  })

  const isTemplate = repositoryDetails?.kind === 'template'

  const modelSelector = useModelSelector<ClaudeModel>(
    isModelSelectorEnabled ? availableModel : DEFAULT_CLAUDE_MODEL,
    isModelSelectorEnabled
      ? onModelChange
      : () => {
          /* no-op: model selector disabled */
        },
    selectedAgent?.models,
    selectedAgent?.defaultModel ?? DEFAULT_CLAUDE_MODEL,
  )
  const agentModeSelector = useAgentModeSelector(agentMode, onAgentModeChange)
  const agentTypeSelector = useAgentTypeSelector(agentType, onAgentTypeChange, agents)

  const workspaceSelector = useWorkspaceSelector(workspaceId, onWorkspaceChange, true)
  const branchSelector = useBranchSelector(
    primaryRepoId,
    branch,
    setBranch,
    true,
    repositoryDetails,
  )

  useEffect(() => {
    if (branchSelector.error) {
      toast.error('Failed to load branches', {
        description: getErrorMessage(branchSelector.error),
      })
    }
  }, [branchSelector.error])

  // Auto-select first workspace when none is selected
  useEffect(() => {
    if (!workspaceId && workspaces && workspaces.length > 0) {
      const firstWorkspace = workspaces.find((workspace) => !workspace.deleted)
      if (firstWorkspace) {
        onWorkspaceChange(firstWorkspace.id)
      }
    }
  }, [workspaceId, workspaces, onWorkspaceChange])

  // Handle message changes with skill autocomplete
  const handleMessageChange = (newMessage: string) => {
    setMessage(newMessage)
    if (textareaRef.current) {
      const cursorPosition = textareaRef.current.selectionStart || 0
      handleCommandInputChange(newMessage, cursorPosition)
    }
  }

  const handleSubmit = () => {
    if (!canSubmit) {
      return
    }

    submit({
      agentType,
      workspaceId,
      fileIds: taskInputs.getFileIds(),
      agentMode,
      message: message.trim() || undefined,
      model: isModelSelectorEnabled ? modelToApiValue(availableModel) : undefined,
      contexts: taskInputs.contextItems,
      skills: taskInputs.selectedSkills,
      branch: branchSelector.value,
      onSubmitted: () => {
        setMessage('')
        taskInputs.clearAll()
        setBranch('')
        onSuccess?.()
      },
    })
  }

  const canSubmit = !!workspaceId && !agentUnavailable

  const placeholder = isTemplate
    ? 'Describe what you want to build...'
    : 'Describe your coding task or leave empty to just run the app...'

  return (
    <div className={className}>
      {showTitle && (
        <div className="text-center mb-4">
          <h1 className="text-4xl font-bold tracking-tight">{title}</h1>
        </div>
      )}

      <div className={showTitle ? 'max-w-2xl mx-auto' : ''} style={{ maxHeight, minHeight }}>
        <ChatInput
          message={message}
          onMessageChange={handleMessageChange}
          onSubmit={handleSubmit}
          placeholder={placeholder}
          canSubmit={canSubmit}
          textareaRef={textareaRef}
          onKeyDownCapture={handleCommandKeyDown}
          onPaste={taskInputs.handlePaste}
          autocompleteSlot={
            isCommandAutocompleteOpen && (
              <CommandAutocomplete
                commands={filteredCommands}
                selectedIndex={commandSelectedIndex}
                onSelect={selectCommand}
                position="above"
              />
            )
          }
        >
          <SkillChips skills={taskInputs.selectedSkills} onRemove={taskInputs.handleRemoveSkill} />

          {taskInputs.contextItems.length > 0 && (
            <ContextItems
              contexts={taskInputs.contextItems}
              onRemove={taskInputs.handleRemoveContext}
            />
          )}

          {taskInputs.uploadedFiles.length > 0 && (
            <FileAttachments
              files={taskInputs.uploadedFiles}
              onRemove={taskInputs.handleRemoveFile}
            />
          )}

          {agentUnavailable && (
            <div className="text-xs text-muted-foreground px-1">
              {selectedAgent?.missingCredentialHint ??
                `Connect ${selectedAgent?.label ?? 'this agent'} to run tasks.`}{' '}
              <button
                type="button"
                className="underline hover:text-foreground"
                onClick={() => setSettingsDialogOpen(true)}
              >
                Open Settings
              </button>
            </div>
          )}

          <div className="flex items-center justify-between gap-2">
            <Selectors>
              <PopoverSelector
                icon={workspaceSelector.icon}
                value={workspaceSelector.value}
                displayValue={workspaceSelector.displayValue}
                tooltip={workspaceSelector.tooltip}
                loading={workspaceSelector.loading}
                disabled={workspaceSelector.disabled}
              >
                <SearchablePopoverContent
                  items={workspaceSelector.items}
                  value={workspaceSelector.value}
                  onChange={workspaceSelector.onChange}
                  placeholder="Search workspaces..."
                  emptyText="No workspaces found"
                />
              </PopoverSelector>

              {!(branchSelector.hidden || isTemplate) && (
                <PopoverSelector
                  icon={branchSelector.icon}
                  value={branchSelector.value}
                  displayValue={branchSelector.displayValue}
                  tooltip={branchSelector.tooltip}
                  loading={branchSelector.loading}
                  disabled={branchSelector.disabled}
                  error={branchSelector.isError}
                >
                  <BranchPopoverContent
                    items={branchSelector.items}
                    value={branchSelector.value}
                    onChange={branchSelector.onChange}
                    currentBranch={branchSelector.currentBranch}
                    enableSearch={true}
                    error={branchSelector.error}
                  />
                </PopoverSelector>
              )}

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

              <PopoverSelector
                icon={agentTypeSelector.icon}
                value={agentTypeSelector.value}
                displayValue={agentTypeSelector.displayValue}
                tooltip={agentTypeSelector.tooltip}
                loading={agentTypeSelector.loading}
                disabled={agentTypeSelector.disabled}
              >
                <SimpleListPopoverContent
                  items={agentTypeSelector.items}
                  value={agentTypeSelector.value}
                  onChange={agentTypeSelector.onChange}
                />
              </PopoverSelector>
            </Selectors>

            <div className="flex items-center gap-2">
              <PopoverSelector
                icon={taskInputs.attachmentSelector.icon}
                value={taskInputs.attachmentSelector.value}
                tooltip={taskInputs.attachmentSelector.tooltip}
                loading={taskInputs.attachmentSelector.loading}
                disabled={taskInputs.attachmentSelector.disabled}
              >
                <AttachmentPopoverContent
                  onFileSelect={(file) => taskInputs.uploadFile.mutate(file)}
                  disabled={taskInputs.uploadFile.isPending}
                />
              </PopoverSelector>
              <SendButton
                disabled={!canSubmit}
                loading={isSubmitting}
                buttonText={getSubmitButtonText(isSubmitting, message)}
              />
            </div>
          </div>
        </ChatInput>
      </div>

      <SettingsDialog
        open={settingsDialogOpen}
        onOpenChange={setSettingsDialogOpen}
        defaultSection={settingsSection}
      />
    </div>
  )
}

function getSubmitButtonText(isPending: boolean, message: string): string {
  if (isPending) {
    return message.trim() ? 'Starting...' : 'Creating...'
  }
  return message.trim() ? 'Start Task' : 'Create Task'
}
