import {
  CLAUDE_MODEL_DISPLAY_NAMES,
  CLAUDE_MODELS,
  type ClaudeModel,
  DEFAULT_CLAUDE_MODEL,
} from '@moatless/types'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useBranches, useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { useTemplatesList } from '@/features/templates'

// Repository selector hook
export function useRepositorySelector(enabled = true, initialValue = '') {
  const [selected, setSelected] = useState(initialValue)
  const { data, isLoading, error, isError } = useRepositoriesList({ enabled })

  const items = useMemo(
    () =>
      data?.map((repo) => ({
        id: repo.id,
        label: repo.name,
      })) || [],
    [data],
  )

  const onChange = useCallback((value: string) => {
    setSelected(value)
  }, [])

  return useMemo(
    () => ({
      value: selected,
      onChange,
      loading: isLoading,
      error,
      isError,
      placeholder: 'Select repository',
      items,
      width: 'w-[180px]',
    }),
    [selected, onChange, isLoading, error, isError, items],
  )
}

// Branch selector hook
export function useBranchSelector(repositoryId?: string, enabled = true) {
  const [selected, setSelected] = useState('')
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

  const onChange = useCallback((value: string) => {
    setSelected(value)
  }, [])

  return useMemo(
    () => ({
      value: selected,
      onChange,
      loading: isLoading,
      error,
      isError,
      placeholder: 'Select branch',
      items,
      width: 'w-[140px]',
      disabled: !repositoryId,
    }),
    [selected, onChange, isLoading, error, isError, items, repositoryId],
  )
}

// Template selector hook
export function useTemplateSelector(enabled = true) {
  const [selected, setSelected] = useState('')
  const { data, isLoading } = useTemplatesList({ enabled })

  const items = useMemo(
    () =>
      data?.templates.map((template) => ({
        id: template.id,
        label: template.name,
      })) || [],
    [data?.templates],
  )

  const onChange = useCallback((value: string) => {
    setSelected(value)
  }, [])

  return useMemo(
    () => ({
      value: selected,
      onChange,
      loading: isLoading,
      placeholder: 'Select template',
      items,
      width: 'w-[180px]',
    }),
    [selected, onChange, isLoading, items],
  )
}

// Model selector hook
export function useModelSelector(initial: ClaudeModel = DEFAULT_CLAUDE_MODEL) {
  const [selected, setSelected] = useState<ClaudeModel>(initial)

  // Sync internal state when initial value changes (e.g., when switching tasks)
  useEffect(() => {
    setSelected(initial)
  }, [initial])

  const items = useMemo(
    () => [
      { id: CLAUDE_MODELS.OPUS_4, label: CLAUDE_MODEL_DISPLAY_NAMES[CLAUDE_MODELS.OPUS_4] },
      { id: CLAUDE_MODELS.SONNET_4, label: CLAUDE_MODEL_DISPLAY_NAMES[CLAUDE_MODELS.SONNET_4] },
    ],
    [],
  )

  const onChange = useCallback((value: string) => {
    setSelected(value as ClaudeModel)
  }, [])

  return useMemo(
    () => ({
      value: selected,
      onChange,
      placeholder: 'Select model',
      items,
      width: 'w-[130px]',
    }),
    [selected, onChange, items],
  )
}

// Agent mode selector hook
export function useAgentModeSelector(initial: 'edit' | 'plan' = 'edit') {
  const [selected, setSelected] = useState<'edit' | 'plan'>(initial)

  // Sync internal state when initial value changes (e.g., when switching tasks)
  useEffect(() => {
    setSelected(initial)
  }, [initial])

  const items = useMemo(
    () => [
      { id: 'edit', label: 'Edit Mode' },
      { id: 'plan', label: 'Plan Mode' },
    ],
    [],
  )

  const onChange = useCallback((value: string) => {
    setSelected(value as 'edit' | 'plan')
  }, [])

  return useMemo(
    () => ({
      value: selected,
      onChange,
      placeholder: 'Mode',
      items,
      width: 'w-[100px]',
    }),
    [selected, onChange, items],
  )
}
