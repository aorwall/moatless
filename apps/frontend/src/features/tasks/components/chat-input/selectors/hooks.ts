import {
  CLAUDE_MODEL_DISPLAY_NAMES,
  CLAUDE_MODELS,
  type ClaudeModel,
  DEFAULT_CLAUDE_MODEL,
} from '@moatless/types'
import { useCallback, useMemo, useState } from 'react'
import { useBranches, useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
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
    }),
    [currentValue, handleChange, isLoading, error, isError, items],
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
    }),
    [derivedValue, handleChange, isLoading, error, isError, items, repositoryId],
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

  return useMemo(
    () => ({
      value: derivedValue,
      onChange: handleChange,
      loading: isLoading,
      placeholder: 'Select template',
      items,
      width: 'w-[180px]',
    }),
    [derivedValue, handleChange, isLoading, items],
  )
}

// Model selector hook - now controlled
export function useModelSelector(value?: ClaudeModel, onChange?: (value: ClaudeModel) => void) {
  const items = useMemo(
    () => [
      { id: CLAUDE_MODELS.OPUS_4, label: CLAUDE_MODEL_DISPLAY_NAMES[CLAUDE_MODELS.OPUS_4] },
      { id: CLAUDE_MODELS.SONNET_4, label: CLAUDE_MODEL_DISPLAY_NAMES[CLAUDE_MODELS.SONNET_4] },
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

  return useMemo(
    () => ({
      value: currentValue,
      onChange: handleChange,
      placeholder: 'Select model',
      items,
      width: 'w-[130px]',
    }),
    [currentValue, handleChange, items],
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

  return useMemo(
    () => ({
      value: currentValue,
      onChange: handleChange,
      placeholder: 'Mode',
      items,
      width: 'w-[100px]',
    }),
    [currentValue, handleChange, items],
  )
}
