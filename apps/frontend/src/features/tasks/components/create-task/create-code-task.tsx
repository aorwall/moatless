import type { RepositoryResponse } from '@moatless/api-client'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useAgentAvailability } from '@/features/agents'
import { useCommandsList } from '@/features/commands/hooks/use-commands'
import { CreateRepositoryDialog, useRepositoriesList, useRepository } from '@/features/repositories'
import { accessFailureLabel, isAccessFailure } from '@/features/repositories/utils/access-status'
import { SettingsDialog } from '@/features/settings'
import { useRepositorySkills } from '@/features/skills'
import {
  ChatInput,
  ContextItems,
  FileAttachments,
  Selectors,
  SendButton,
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
  useRepositorySelector,
  useTaskTemplateSelector,
} from '@/features/tasks/components/chat/chat-input/selectors'
import {
  CommandAutocomplete,
  useCommandAutocomplete,
} from '@/features/tasks/components/chat/command-autocomplete'
import {
  type AdditionalRepoDraft,
  AdditionalReposField,
} from '@/features/tasks/components/create-task/additional-repos-field'
import { useCodeTaskSubmit } from '@/features/tasks/components/create-task/use-code-task-submit'
import { useTaskInputs } from '@/features/tasks/hooks/use-task-inputs'
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
  repositoryId: string
  onRepositoryChange: (repoId: string) => void
  templateId?: string | undefined
  onTemplateChange?: ((id: string | undefined) => void) | undefined
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
  repositoryId,
  onRepositoryChange,
  templateId,
  onTemplateChange,
  onSuccess,
}: CreateCodeTaskProps) {
  const { submit, isSubmitting } = useCodeTaskSubmit()
  const [message, setMessage] = useState(defaultMessage || '')
  const [branch, setBranch] = useState('')
  const [additionalRepos, setAdditionalRepos] = useState<AdditionalRepoDraft[]>([])
  const [repoDialogOpen, setRepoDialogOpen] = useState(false)
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false)

  // Backend-driven agent catalog (availability + per-agent model list),
  // plus model-reset-on-agent-change effect.
  const { agents, selectedAgent, agentUnavailable, settingsSection, availableModel } =
    useAgentAvailability({ agentType, model })

  // Fetch global commands for slash command autocomplete
  const { data: commands = [] } = useCommandsList()

  // Fetch repo-scoped skills (workspace + linked plugin repos). Only fetch
  // once a repository is selected — the composer can't submit without one.
  const { data: skills = [], error: skillsError } = useRepositorySkills(repositoryId || undefined)

  // Surface skill-fetch failures via a toast. The autocomplete still works
  // with just the global commands list.
  useEffect(() => {
    if (skillsError) {
      toast.error('Failed to load repository skills', {
        description: getErrorMessage(skillsError),
      })
    }
  }, [skillsError])

  // Merge commands + skills into a single Suggestion[] for the "/" dropdown.
  const suggestions = [
    ...commands.map((c) => ({ kind: 'command' as const, ...c })),
    ...skills.map((s) => ({ kind: 'skill' as const, ...s })),
  ]

  // Command autocomplete hook
  const {
    inputRef: textareaRef,
    isOpen: isCommandAutocompleteOpen,
    filteredCommands,
    selectedIndex: commandSelectedIndex,
    selectedCommand,
    handleInputChange: handleCommandInputChange,
    handleKeyDown: handleCommandKeyDown,
    selectCommand,
    clearSelectedCommand,
  } = useCommandAutocomplete({
    onValueChange: setMessage,
    suggestions,
    enabled: true,
  })

  // Check if model selector should be shown
  const isModelSelectorEnabled = useFeatureFlag('SHOW_MODEL_SELECTOR')

  // Task inputs hook (file uploads, context items)
  const taskInputs = useTaskInputs()

  // Fetch repositories list to detect when new repositories are added
  const { data: repositories } = useRepositoriesList()

  // Fetch full repository details to check worktree/clone settings
  const { data: repositoryDetails, error: repositoryError } = useRepository(repositoryId, {
    enabled: !!repositoryId,
  })

  const isTemplate = repositoryDetails?.kind === 'template'

  // Clear stored repository ID on 404 error
  useEffect(() => {
    if (isNotFoundError(repositoryError)) {
      onRepositoryChange('')
      toast.info('Previously selected repository no longer exists')
    }
  }, [repositoryError, onRepositoryChange])

  // Use controlled selectors
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

  const repoSelector = useRepositorySelector(repositoryId, onRepositoryChange, true)
  const branchSelector = useBranchSelector(repositoryId, branch, setBranch, true, repositoryDetails)
  const templateSelector = useTaskTemplateSelector(templateId, onTemplateChange)

  // Surface branch-fetch failures via a toast (e.g., 403 when token lacks access)
  useEffect(() => {
    if (branchSelector.error) {
      toast.error('Failed to load branches', {
        description: getErrorMessage(branchSelector.error),
      })
    }
  }, [branchSelector.error])

  // Auto-select first repository when dialog closes and a repository is available but none selected
  useEffect(() => {
    const firstRepo = repositories?.[0]
    if (!(repoDialogOpen || repositoryId) && firstRepo) {
      onRepositoryChange(firstRepo.id)
    }
  }, [repoDialogOpen, repositoryId, repositories, onRepositoryChange])

  // Handle message changes with command autocomplete
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

    const { submitMessage, submitAgentMode } = resolveCommandSubmit(
      message,
      agentMode,
      selectedCommand,
    )

    submit({
      agentType,
      repositoryId,
      additionalRepos,
      repositories,
      fileIds: taskInputs.getFileIds(),
      agentMode: submitAgentMode,
      message: submitMessage,
      model: isModelSelectorEnabled ? modelToApiValue(availableModel) : undefined,
      contexts: taskInputs.contextItems,
      branch: branchSelector.value,
      commandName: selectedCommand?.name,
      templateId,
      onSubmitted: () => {
        setMessage('')
        clearSelectedCommand()
        taskInputs.clearAll()
        setBranch('')
        setAdditionalRepos([])
        onSuccess?.()
      },
    })
  }

  const canSubmit = !!repositoryId && !agentUnavailable

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

          <RepoAccessWarning repository={repositoryDetails} />

          <div className="flex items-center justify-between gap-2">
            <Selectors>
              {templateSelector.items.length > 0 && (
                <PopoverSelector
                  icon={templateSelector.icon}
                  value={templateSelector.value}
                  displayValue={templateSelector.displayValue}
                  tooltip={templateSelector.tooltip}
                  loading={templateSelector.loading}
                  disabled={templateSelector.disabled}
                >
                  <SimpleListPopoverContent
                    items={templateSelector.items}
                    value={templateSelector.value}
                    onChange={templateSelector.onChange}
                  />
                </PopoverSelector>
              )}

              <PopoverSelector
                icon={repoSelector.icon}
                value={repoSelector.value}
                displayValue={repoSelector.displayValue}
                tooltip={repoSelector.tooltip}
                loading={repoSelector.loading}
                disabled={repoSelector.disabled}
              >
                <SearchablePopoverContent
                  items={repoSelector.items}
                  value={repoSelector.value}
                  onChange={repoSelector.onChange}
                  placeholder="Search repositories..."
                  emptyText="No repositories found"
                  onEmptyAction={() => setRepoDialogOpen(true)}
                  emptyActionText="Create Repository"
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

      <AdditionalReposField
        primaryRepositoryId={repositoryId}
        isTemplate={isTemplate}
        showTitle={showTitle}
        value={additionalRepos}
        onChange={setAdditionalRepos}
        disabled={isSubmitting}
      />

      <CreateRepositoryDialog
        open={repoDialogOpen}
        onOpenChange={setRepoDialogOpen}
        onCreated={(repositoryId) => repoSelector.onChange(repositoryId)}
      />

      <SettingsDialog
        open={settingsDialogOpen}
        onOpenChange={setSettingsDialogOpen}
        defaultSection={settingsSection}
      />
    </div>
  )
}

// Warn when the selected repository has a provider-reachability failure — its
// tasks would fail to clone. Renders nothing for healthy/unloaded repos.
function RepoAccessWarning({ repository }: { repository: RepositoryResponse | undefined }) {
  if (!(repository && isAccessFailure(repository.accessStatus))) {
    return null
  }
  return (
    <div className="text-xs text-destructive px-1">
      {accessFailureLabel(repository.accessStatus)}: this repository may not be reachable with your
      current credentials.
      {repository.accessError ? ` ${repository.accessError}` : ''}
    </div>
  )
}

// A fetch error that carries an HTTP 404 status (repository no longer exists).
function isNotFoundError(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'status' in error &&
    (error as { status: number }).status === 404
  )
}

function getSubmitButtonText(isPending: boolean, message: string): string {
  if (isPending) {
    return message.trim() ? 'Starting...' : 'Creating...'
  }
  return message.trim() ? 'Start Task' : 'Create Task'
}

// When a `/command` is selected, the backend populates the prompt — we only
// submit the user's trailing text and optionally switch agent mode.
function resolveCommandSubmit(
  message: string,
  agentMode: 'edit' | 'plan',
  selectedCommand: { name: string; agentMode?: string | null } | null,
): { submitMessage: string | undefined; submitAgentMode: 'edit' | 'plan' } {
  if (!selectedCommand) {
    return { submitMessage: message.trim() || undefined, submitAgentMode: agentMode }
  }
  const prefix = `/${selectedCommand.name} `
  const userText = message.startsWith(prefix) ? message.slice(prefix.length).trim() : ''
  return {
    submitMessage: userText || undefined,
    submitAgentMode: (selectedCommand.agentMode as 'edit' | 'plan') ?? agentMode,
  }
}
