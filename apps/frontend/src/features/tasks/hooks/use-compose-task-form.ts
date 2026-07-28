import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useAgentAvailability } from '@/features/agents'
import { useRepository } from '@/features/repositories'
import { useRepositorySkills } from '@/features/skills'
import {
  type AgentType,
  useAgentModeSelector,
  useAgentTypeSelector,
  useBranchSelector,
  useModelSelector,
  useWorkspaceSelector,
} from '@/features/tasks/components/chat/chat-input/selectors'
import { useCommandAutocomplete } from '@/features/tasks/components/chat/command-autocomplete'
import { useTaskInputs } from '@/features/tasks/hooks/use-task-inputs'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { useWorkspace, useWorkspacesList } from '@/features/workspaces/hooks/use-workspaces'
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
  workspaceId: string
  onWorkspaceChange: (value: string) => void
  model: ClaudeModel
  onModelChange: (value: ClaudeModel) => void
  agentMode: 'edit' | 'plan'
  onAgentModeChange: (value: 'edit' | 'plan') => void
  agentType: AgentType
  onAgentTypeChange: (value: AgentType) => void
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
    workspaceId,
    onWorkspaceChange,
    model,
    onModelChange,
    agentMode,
    onAgentModeChange,
    agentType,
    onAgentTypeChange,
    navigateOnSuccess = true,
    onSuccess,
  } = args

  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const [settingsDialogOpen, setSettingsDialogOpen] = useState(false)

  const { agents, selectedAgent, agentUnavailable, settingsSection, availableModel } =
    useAgentAvailability({ agentType, model })

  // Derive primary repo from the selected workspace for branch/skills
  const { data: workspaceDetails } = useWorkspace(workspaceId, { enabled: !!workspaceId })
  const primaryRepoId = workspaceDetails?.repos.find((r) => r.isPrimary)?.repositoryId ?? undefined

  const { data: skills = [], error: skillsError } = useRepositorySkills(primaryRepoId)

  useEffect(() => {
    if (skillsError) {
      toast.error('Failed to load repository skills', {
        description: getErrorMessage(skillsError),
      })
    }
  }, [skillsError])

  const suggestions = skills.map((s) => ({ kind: 'skill' as const, ...s }))

  const {
    inputRef: textareaRef,
    isOpen: isCommandAutocompleteOpen,
    filteredCommands,
    selectedIndex: commandSelectedIndex,
    handleInputChange: handleCommandInputChange,
    handleKeyDown: handleCommandKeyDown,
    selectCommand,
  } = useCommandAutocomplete({
    onValueChange: onMessageChange,
    suggestions,
    enabled: true,
  })

  const isModelSelectorEnabled = useFeatureFlag('SHOW_MODEL_SELECTOR')

  const taskInputs = useTaskInputs()

  const { data: workspaces } = useWorkspacesList()
  const { data: repositoryDetails, error: repositoryError } = useRepository(primaryRepoId ?? '', {
    enabled: !!primaryRepoId,
  })

  const isTemplate = repositoryDetails?.kind === 'template'

  useEffect(() => {
    if (
      repositoryError &&
      'status' in repositoryError &&
      (repositoryError as { status: number }).status === 404
    ) {
      onWorkspaceChange('')
      toast.info('Previously selected workspace is no longer available')
    }
  }, [repositoryError, onWorkspaceChange])

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
    onBranchChange,
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

  const handleMessageChange = (newMessage: string) => {
    onMessageChange(newMessage)
    if (textareaRef.current) {
      const cursorPosition = textareaRef.current.selectionStart || 0
      handleCommandInputChange(newMessage, cursorPosition)
    }
  }

  const canSubmit = !!workspaceId && !agentUnavailable

  const handleSubmit = () => {
    if (!canSubmit) {
      return
    }

    const submitMessage = message.trim() || undefined

    const trimmedName = name.trim()
    const modelValue = isModelSelectorEnabled ? modelToApiValue(availableModel) : undefined
    createTask.mutate(
      {
        agentType,
        workspaceId,
        agentMode,
        fileIds: taskInputs.getFileIds(),
        ...(trimmedName !== '' && { name: trimmedName }),
        ...(submitMessage !== undefined && { message: submitMessage }),
        ...(modelValue !== undefined && { model: modelValue }),
        ...(taskInputs.contextItems.length > 0 && { contexts: taskInputs.contextItems }),
        ...(branchSelector.value !== undefined && { branch: branchSelector.value }),
      },
      {
        onSuccess: () => {
          taskInputs.clearAll()
          onSuccess?.()
        },
      },
    )
  }

  const placeholder = isTemplate
    ? 'Describe what you want to build…'
    : 'Describe your coding task or leave empty to just run the app…'

  return {
    // selectors
    workspaceSelector,
    branchSelector,
    modelSelector,
    agentModeSelector,
    agentTypeSelector,
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
    // submit
    handleSubmit,
    canSubmit,
    isPending: createTask.isPending,
    // dialogs
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
