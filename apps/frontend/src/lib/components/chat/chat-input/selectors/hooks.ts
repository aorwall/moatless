import {
  CLAUDE_MODEL_DISPLAY_NAMES,
  CLAUDE_MODELS,
  type ClaudeModel,
  DEFAULT_CLAUDE_MODEL,
} from '@moatless/types'
import { Bot, Database, FileText, GitBranch, Wand2 } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { useBranches, useRepositoriesList } from '@/features/repositories/hooks/use-repositories.ts'
import { useTemplatesList } from '@/features/templates'

// Repository selector hook - now controlled
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
) {
  const { data, isLoading, error, isError } = useBranches(repositoryId || '', {
    enabled: enabled && !!repositoryId,
  })

  const items = useMemo(
    () =>
      data?.branches.map((branch) => ({
        id: branch,
        label: branch,
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
      return items[0].id
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
      // New popover selector props
      icon: GitBranch,
      tooltip: 'Choose a branch',
      displayValue: selectedBranch?.label || 'Select branch',
      currentBranch: data?.currentBranch,
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
      data?.currentBranch,
    ],
  )
}

// Template selector hook - now controlled
export function useTemplateSelector(
  value?: string,
  onChange?: (value: string) => void,
  enabled = true,
) {
  const { data, isLoading } = useTemplatesList({ enabled })

  const items = useMemo(
    () =>
      data?.templates.map((template) => ({
        id: template.id,
        label: template.name,
      })) || [],
    [data?.templates],
  )

  // Internal state for uncontrolled mode
  const [internalValue, setInternalValue] = useState('')
  const isControlled = value !== undefined
  const currentValue = isControlled ? value : internalValue

  // Auto-select first template if no value is set
  const derivedValue = useMemo(() => {
    if (!currentValue && items.length > 0) {
      return items[0].id
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

  const selectedTemplate = useMemo(
    () => items.find((item) => item.id === derivedValue),
    [items, derivedValue],
  )

  return useMemo(
    () => ({
      value: derivedValue,
      onChange: handleChange,
      loading: isLoading,
      placeholder: 'Select template',
      items,
      width: 'w-[180px]',
      // New popover selector props
      icon: FileText,
      tooltip: 'Choose a template',
      displayValue: selectedTemplate?.label || 'Select template',
    }),
    [derivedValue, handleChange, isLoading, items, selectedTemplate],
  )
}

// Model selector hook - now controlled
export function useModelSelector(value?: ClaudeModel, onChange?: (value: ClaudeModel) => void) {
  const items = useMemo(
    () => [
      {
        id: CLAUDE_MODELS.OPUS_4,
        label: CLAUDE_MODEL_DISPLAY_NAMES[CLAUDE_MODELS.OPUS_4],
      },
      {
        id: CLAUDE_MODELS.SONNET_4,
        label: CLAUDE_MODEL_DISPLAY_NAMES[CLAUDE_MODELS.SONNET_4],
      },
    ],
    [],
  )

  // Internal state for uncontrolled mode
  const [internalValue, setInternalValue] = useState<ClaudeModel>(DEFAULT_CLAUDE_MODEL)
  const isControlled = value !== undefined
  const currentValue = isControlled ? value : internalValue

  const handleChange = useCallback(
    (newValue: string) => {
      const modelValue = newValue as ClaudeModel
      if (!isControlled) {
        setInternalValue(modelValue)
      }
      onChange?.(modelValue)
    },
    [isControlled, onChange],
  )

  const selectedModel = useMemo(
    () => items.find((item) => item.id === currentValue),
    [items, currentValue],
  )

  return useMemo(
    () => ({
      value: currentValue,
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
