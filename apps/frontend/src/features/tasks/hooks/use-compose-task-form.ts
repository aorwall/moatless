import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { useAgentAvailability } from '@/features/agents'
import { useCommandsList } from '@/features/commands/hooks/use-commands'
import { useRepositoriesList, useRepository } from '@/features/repositories'
import { useRepositorySkills } from '@/features/skills'
import {
  type AgentType,
  useAgentModeSelector,
  useAgentTypeSelector,
  useBranchSelector,
  useModelSelector,
  useRepositorySelector,
  useTaskTemplateSelector,
} from '@/features/tasks/components/chat/chat-input/selectors'
import {
  type Suggestion,
  useCommandAutocomplete,
} from '@/features/tasks/components/chat/command-autocomplete'
import { useTaskInputs } from '@/features/tasks/hooks/use-task-inputs'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { queryKeys } from '@/lib/api/query-keys'
import { useFeatureFlag } from '@/lib/hooks/use-feature-flag'
import { type ClaudeModel, DEFAULT_CLAUDE_MODEL, modelToApiValue } from '@/lib/types/claude-models'
import { getErrorMessage } from '@/lib/utils/error'

export interface UseComposeTaskFormArgs {
  name: string
  onNameChange: (value: string) => void
  message: string
  onMessageChange: (value: string) => void
  branch: string
  onBranchChange: (value: string) => void
  repositoryId: string
  onRepositoryChange: (value: string) => void
  model: ClaudeModel
  onModelChange: (value: ClaudeModel) => void
  agentMode: 'edit' | 'plan'
  onAgentModeChange: (value: 'edit' | 'plan') => void
  agentType: AgentType
  onAgentTypeChange: (value: AgentType) => void
  templateId?: string | undefined
  onTemplateChange?: ((value: string | undefined) => void) | undefined
  /** Navigate to the created task, or stay put with a toast (docked compose). */
  navigateOnSuccess?: boolean | undefined
  /** Fired after a task is successfully created (e.g. to close the window). */
  onSuccess?: (() => void) | undefined
}

/**
 * All the wiring for the Gmail-style compose form — selectors, slash-command
 * autocomplete, attachments, and task creation — extracted so the component
 * stays presentational. Mirrors the behavior of `CreateCodeTask`.
 */
export function useComposeTaskForm(args: UseComposeTaskFormArgs) {
  const {
    name,
    message,
    onMessageChange,
    branch,
    onBranchChange,
    repositoryId,
    onRepositoryChange,
    model,
    onModelChange,
    agentMode,
    onAgentModeChange,
    agentType,
    onAgentTypeChange,
    templateId,
    onTemplateChange,
    navigateOnSuccess = true,
    onSuccess,
  } = args

  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const [repoDialogOpen, setRepoDialogOpen] = useState(false)
  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false)

  const { agents, selectedAgent, agentUnavailable, settingsSection } = useAgentAvailability({
    agentType,
    model,
    onModelChange,
  })

  const { data: commands = [] } = useCommandsList()
  const { data: skills = [], error: skillsError } = useRepositorySkills(repositoryId || undefined)

  useEffect(() => {
    if (skillsError) {
      toast.error('Failed to load repository skills', {
        description: getErrorMessage(skillsError),
      })
    }
  }, [skillsError])

  const suggestions = useMemo<Suggestion[]>(
    () => [
      ...commands.map((c) => ({ kind: 'command' as const, ...c })),
      ...skills.map((s) => ({ kind: 'skill' as const, ...s })),
    ],
    [commands, skills],
  )

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
    onValueChange: onMessageChange,
    suggestions,
    enabled: true,
  })

  const isModelSelectorEnabled = useFeatureFlag('SHOW_MODEL_SELECTOR')

  const taskInputs = useTaskInputs({
    enableContextItems: true,
    enableFigma: true,
  })

  const { data: repositories } = useRepositoriesList()
  const { data: repositoryDetails, error: repositoryError } = useRepository(repositoryId, {
    enabled: !!repositoryId,
  })

  const isTemplate = repositoryDetails?.kind === 'template'

  useEffect(() => {
    if (
      repositoryError &&
      'status' in repositoryError &&
      (repositoryError as { status: number }).status === 404
    ) {
      onRepositoryChange('')
      toast.info('Previously selected repository no longer exists')
    }
  }, [repositoryError, onRepositoryChange])

  const modelSelector = useModelSelector<ClaudeModel>(
    isModelSelectorEnabled ? model : DEFAULT_CLAUDE_MODEL,
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
  const branchSelector = useBranchSelector(
    repositoryId,
    branch,
    onBranchChange,
    true,
    repositoryDetails,
  )
  const templateSelector = useTaskTemplateSelector(templateId, onTemplateChange)

  useEffect(() => {
    if (branchSelector.error) {
      toast.error('Failed to load branches', {
        description: getErrorMessage(branchSelector.error),
      })
    }
  }, [branchSelector.error])

  useEffect(() => {
    if (!(repoDialogOpen || repositoryId) && repositories && repositories.length > 0) {
      const firstRepo = repositories[0]
      if (firstRepo) {
        onRepositoryChange(firstRepo.id)
      }
    }
  }, [repoDialogOpen, repositoryId, repositories, onRepositoryChange])

  const createTask = useCreateTask({
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.tasks.detail(data.taskId) })
      notifyTaskCreated(data.taskId, navigateOnSuccess, navigate)
    },
    onError: (error) => {
      toast.error('Failed to create task', {
        description: getErrorMessage(error),
      })
    },
  })

  const handleMessageChange = useCallback(
    (newMessage: string) => {
      onMessageChange(newMessage)
      if (textareaRef.current) {
        const cursorPosition = textareaRef.current.selectionStart || 0
        handleCommandInputChange(newMessage, cursorPosition)
      }
    },
    [handleCommandInputChange, textareaRef, onMessageChange],
  )

  const canSubmit = !!repositoryId && !agentUnavailable

  const handleSubmit = useCallback(() => {
    if (!canSubmit) {
      return
    }

    const { submitMessage, submitAgentMode } = resolveCommandSubmit(
      message,
      agentMode,
      selectedCommand,
    )

    const trimmedName = name.trim()
    const modelValue = isModelSelectorEnabled ? modelToApiValue(model) : undefined
    createTask.mutate(
      {
        agentType,
        repositoryId,
        agentMode: submitAgentMode,
        fileIds: taskInputs.getFileIds(),
        ...(trimmedName !== '' && { name: trimmedName }),
        ...(submitMessage !== undefined && { message: submitMessage }),
        ...(modelValue !== undefined && { model: modelValue }),
        ...(taskInputs.contextItems.length > 0 && { contexts: taskInputs.contextItems }),
        ...(branchSelector.value !== undefined && { branch: branchSelector.value }),
        ...(selectedCommand?.name !== undefined && { commandName: selectedCommand.name }),
        ...(templateId !== undefined && { taskTemplateId: templateId }),
      },
      {
        onSuccess: () => {
          clearSelectedCommand()
          taskInputs.clearAll()
          onSuccess?.()
        },
      },
    )
  }, [
    canSubmit,
    message,
    agentMode,
    selectedCommand,
    createTask,
    agentType,
    name,
    repositoryId,
    isModelSelectorEnabled,
    model,
    taskInputs,
    branchSelector.value,
    templateId,
    clearSelectedCommand,
    onSuccess,
  ])

  const handleFigmaImportClick = useCallback(() => {
    taskInputs.setFigmaDialogOpen(true)
  }, [taskInputs])

  const placeholder = isTemplate
    ? 'Describe what you want to build…'
    : 'Describe your coding task or leave empty to just run the app…'

  return {
    // selectors
    repoSelector,
    branchSelector,
    modelSelector,
    agentModeSelector,
    agentTypeSelector,
    templateSelector,
    isModelSelectorEnabled,
    isTemplate,
    // agent availability
    agentUnavailable,
    selectedAgent,
    settingsSection,
    // autocomplete + message
    textareaRef,
    isCommandAutocompleteOpen,
    filteredCommands,
    commandSelectedIndex,
    selectCommand,
    handleCommandKeyDown,
    handleMessageChange,
    placeholder,
    // attachments
    taskInputs,
    handleFigmaImportClick,
    // submit
    handleSubmit,
    canSubmit,
    isPending: createTask.isPending,
    // dialogs
    repoDialogOpen,
    setRepoDialogOpen,
    settingsDialogOpen,
    setSettingsDialogOpen,
  }
}

// After a task is created, either navigate to it (default) or — for the docked
// compose experience — stay put and surface a toast linking to the new task.
function notifyTaskCreated(
  taskId: string,
  navigateOnSuccess: boolean,
  navigate: ReturnType<typeof useNavigate>,
) {
  const go = () => navigate({ to: '/tasks/$task-id', params: { 'task-id': taskId } })
  if (navigateOnSuccess) {
    go()
    return
  }
  toast.success('Task created', {
    action: { label: 'View task', onClick: go },
  })
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
