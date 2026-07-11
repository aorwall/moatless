import { Bot, Database, GitBranch, LayoutTemplate, Plus, Sparkles, Wand2 } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import type { AgentDescriptor, AgentModel } from '@/features/agents/types'
import { useBranches, useRepositoriesList } from '@/features/repositories/hooks/use-repositories.ts'
import type { TaskTemplateResponse } from '@/features/task-templates/api/task-templates'
import { useTaskTemplatesList } from '@/features/task-templates/hooks/use-task-templates'
import {
  CLAUDE_MODEL_DISPLAY_NAMES,
  CLAUDE_MODELS,
  DEFAULT_CLAUDE_MODEL,
} from '@/lib/types/claude-models'

// Repository selector hook - now controlled
// Shows all repositories (including templates)
export function useRepositorySelector(
  value?: string,
  onChange?: (value: string) => void,
  enabled = true,
) {
  const { data, isLoading, error, isError } = useRepositoriesList({ enabled })

  const items = useMemo(
    () =>
      data?.map((repo) => ({
        id: repo.id,
        label: repo.name,
      })) || [],
    [data],
  )

  // Internal state for uncontrolled mode
  const [internalValue, setInternalValue] = useState('')
  const isControlled = value !== undefined
  const currentValue = isControlled ? value : internalValue

  const handleChange = useCallback(
    (newValue: string) => {
      if (!isControlled) {
        setInternalValue(newValue)
      }
      onChange?.(newValue)
    },
    [isControlled, onChange],
  )

  const selectedRepo = useMemo(
    () => data?.find((repo) => repo.id === currentValue),
    [data, currentValue],
  )

  return useMemo(
    () => ({
      value: currentValue,
      onChange: handleChange,
      loading: isLoading,
      error,
      isError,
      placeholder: 'Select repository',
      items,
      width: 'w-[180px]',
      disabled: false,
      // New popover selector props
      icon: Database,
      tooltip: 'Choose a repository',
      displayValue: selectedRepo?.name || 'Select repository',
    }),
    [currentValue, handleChange, isLoading, error, isError, items, selectedRepo],
  )
}

// Branch selector hook - now controlled
export function useBranchSelector(
  repositoryId?: string,
  value?: string,
  onChange?: (value: string) => void,
  enabled = true,
  repository?: {
    kind: string
    path?: string | null
    useGitWorktrees?: boolean | null
  },
) {
  const { data, isLoading, error, isError } = useBranches(repositoryId || '', {
    enabled: enabled && !!repositoryId,
  })

  const items = useMemo(
    () =>
      data?.branches.map((branch) => ({
        id: branch.name,
        label: branch.name,
      })) || [],
    [data?.branches],
  )

  // Internal state for uncontrolled mode
  const [internalValue, setInternalValue] = useState('')
  const isControlled = value !== undefined
  const currentValue = isControlled ? value : internalValue

  // Auto-select first branch if no value is set
  const derivedValue = useMemo(() => {
    if (!currentValue && items.length > 0) {
      const firstItem = items[0]
      return firstItem?.id
    }
    return currentValue
  }, [currentValue, items])

  const handleChange = useCallback(
    (newValue: string) => {
      if (!isControlled) {
        setInternalValue(newValue)
      }
      onChange?.(newValue)
    },
    [isControlled, onChange],
  )

  const selectedBranch = useMemo(
    () => items.find((item) => item.id === derivedValue),
    [items, derivedValue],
  )

  // Determine if branch selector should be hidden
  const hidden = useMemo(() => {
    if (!repository) {
      return false
    }
    // Show branches for:
    // 1. Remote repositories (can select which branch to clone)
    // 2. Repositories using git worktrees (each task gets its own worktree with selectable branch)
    // Hide for local/template repos without worktrees (they use the current branch)
    return !(repository.kind === 'remote' || repository.useGitWorktrees)
  }, [repository])

  return useMemo(
    () => ({
      value: derivedValue,
      onChange: handleChange,
      loading: isLoading,
      error,
      isError,
      placeholder: 'Select branch',
      items,
      width: 'w-[140px]',
      disabled: !repositoryId,
      hidden,
      // New popover selector props
      icon: GitBranch,
      tooltip: 'Choose a branch',
      displayValue: selectedBranch?.label || 'Select branch',
      currentBranch: data?.defaultBranch ?? undefined,
    }),
    [
      derivedValue,
      handleChange,
      isLoading,
      error,
      isError,
      items,
      repositoryId,
      selectedBranch,
      data?.defaultBranch,
      hidden,
    ],
  )
}

const FALLBACK_MODELS: AgentModel[] = [
  { id: CLAUDE_MODELS.DEFAULT, label: CLAUDE_MODEL_DISPLAY_NAMES[CLAUDE_MODELS.DEFAULT] },
  { id: CLAUDE_MODELS.OPUS, label: CLAUDE_MODEL_DISPLAY_NAMES[CLAUDE_MODELS.OPUS] },
  { id: CLAUDE_MODELS.SONNET, label: CLAUDE_MODEL_DISPLAY_NAMES[CLAUDE_MODELS.SONNET] },
]

// Model selector hook - now controlled.
// When `models`/`defaultModelId` are omitted, falls back to the Claude model list
// for backwards compatibility with call-sites that haven't adopted the agents API yet.
export function useModelSelector<T extends string = string>(
  value?: T,
  onChange?: (value: T) => void,
  models?: AgentModel[],
  defaultModelId?: string,
) {
  const items = useMemo<AgentModel[]>(() => {
    if (models && models.length > 0) {
      return models
    }
    return FALLBACK_MODELS
  }, [models])

  const fallbackDefault = defaultModelId ?? DEFAULT_CLAUDE_MODEL

  // Internal state for uncontrolled mode
  const [internalValue, setInternalValue] = useState<string>(fallbackDefault)
  const isControlled = value !== undefined
  const currentValue = isControlled ? value : internalValue

  const handleChange = useCallback(
    (newValue: string) => {
      if (!isControlled) {
        setInternalValue(newValue)
      }
      onChange?.(newValue as T)
    },
    [isControlled, onChange],
  )

  const selectedModel = useMemo(
    () => items.find((item) => item.id === currentValue),
    [items, currentValue],
  )

  return useMemo(
    () => ({
      value: currentValue as string,
      onChange: handleChange,
      placeholder: 'Select model',
      items,
      width: 'w-[130px]',
      loading: false,
      disabled: false,
      // New popover selector props
      icon: Bot,
      tooltip: 'Choose AI model',
      displayValue: selectedModel?.label || 'Select model',
    }),
    [currentValue, handleChange, items, selectedModel],
  )
}

// Agent mode selector hook - now controlled
export function useAgentModeSelector(
  value?: 'edit' | 'plan',
  onChange?: (value: 'edit' | 'plan') => void,
) {
  const items = useMemo(
    () => [
      { id: 'edit', label: 'Edit Mode' },
      { id: 'plan', label: 'Plan Mode' },
    ],
    [],
  )

  // Internal state for uncontrolled mode
  const [internalValue, setInternalValue] = useState<'edit' | 'plan'>('edit')
  const isControlled = value !== undefined
  const currentValue = isControlled ? value : internalValue

  const handleChange = useCallback(
    (newValue: string) => {
      const modeValue = newValue as 'edit' | 'plan'
      if (!isControlled) {
        setInternalValue(modeValue)
      }
      onChange?.(modeValue)
    },
    [isControlled, onChange],
  )

  const selectedMode = useMemo(
    () => items.find((item) => item.id === currentValue),
    [items, currentValue],
  )

  return useMemo(
    () => ({
      value: currentValue,
      onChange: handleChange,
      placeholder: 'Mode',
      items,
      width: 'w-[100px]',
      loading: false,
      disabled: false,
      // New popover selector props
      icon: Wand2,
      tooltip: 'Choose agent mode',
      displayValue: selectedMode?.label || 'Select mode',
    }),
    [currentValue, handleChange, items, selectedMode],
  )
}

export type AgentType =
  | 'claude-code'
  | 'claude-code-tui'
  | 'codex'
  | 'claude-code-cli'
  | 'claude-code-fable'
export const DEFAULT_AGENT_TYPE: AgentType = 'claude-code'

const DISCONNECTED_DESCRIPTION = 'Not connected — open Settings to add a token'

// Agent type selector hook - controlled. Picks which agent CLI runs the task.
// Accepts optional `agents` list from the backend catalog to drive availability.
export function useAgentTypeSelector(
  value?: AgentType,
  onChange?: (value: AgentType) => void,
  agents?: AgentDescriptor[],
) {
  const [internalValue, setInternalValue] = useState<AgentType>(DEFAULT_AGENT_TYPE)
  const isControlled = value !== undefined
  const currentValue = isControlled ? value : internalValue

  const items = useMemo(() => {
    return (agents ?? []).map((agent) => ({
      id: agent.id as AgentType,
      label: agent.label,
      description: agent.installed ? undefined : DISCONNECTED_DESCRIPTION,
      disabled: !agent.installed,
    }))
  }, [agents])

  const handleChange = useCallback(
    (newValue: string) => {
      const typeValue = newValue as AgentType
      if (!isControlled) {
        setInternalValue(typeValue)
      }
      onChange?.(typeValue)
    },
    [isControlled, onChange],
  )

  const selectedType = useMemo(
    () => items.find((item) => item.id === currentValue),
    [items, currentValue],
  )

  return useMemo(
    () => ({
      value: currentValue,
      onChange: handleChange,
      placeholder: 'Agent',
      items,
      width: 'w-[120px]',
      loading: false,
      disabled: false,
      icon: Sparkles,
      tooltip: 'Choose which agent runs the task',
      displayValue: selectedType?.label || 'Select agent',
    }),
    [currentValue, handleChange, items, selectedType],
  )
}

// Task template selector hook - controlled. Lets the user pick a template from
// the full visible list (user + team + global) before submitting a task.
export function useTaskTemplateSelector(
  value?: string,
  onChange?: (id: string | undefined) => void,
  _templates?: TaskTemplateResponse[],
) {
  const { data: fetchedTemplates, isLoading } = useTaskTemplatesList()
  const templates = _templates ?? fetchedTemplates ?? []

  const [internalValue, setInternalValue] = useState<string | undefined>(undefined)
  const isControlled = value !== undefined
  const currentValue = isControlled ? value : internalValue

  const items = useMemo(
    () =>
      templates.map((t) => ({
        id: t.id,
        label: t.name,
        description: t.description ?? undefined,
      })),
    [templates],
  )

  const handleChange = useCallback(
    (newValue: string) => {
      const nextValue = newValue || undefined
      if (!isControlled) {
        setInternalValue(nextValue)
      }
      onChange?.(nextValue)
    },
    [isControlled, onChange],
  )

  const selectedTemplate = useMemo(
    () => templates.find((t) => t.id === currentValue),
    [templates, currentValue],
  )

  return useMemo(
    () => ({
      value: currentValue ?? '',
      onChange: handleChange,
      placeholder: 'Template',
      items,
      loading: isLoading,
      disabled: false,
      icon: LayoutTemplate,
      tooltip: 'Choose a task template',
      displayValue: selectedTemplate?.name ?? 'Template',
    }),
    [currentValue, handleChange, items, isLoading, selectedTemplate],
  )
}

// Attachment selector hook
export function useAttachmentSelector(disabled = false, loading = false) {
  return useMemo(
    () => ({
      value: '',
      // biome-ignore lint/suspicious/noEmptyBlockStatements: Actions handled in popover content
      onChange: () => {},
      loading,
      disabled,
      // Popover selector props
      icon: Plus,
      tooltip: 'Add attachments',
      displayValue: '',
    }),
    [disabled, loading],
  )
}
