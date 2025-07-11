import {
  CLAUDE_MODEL_DISPLAY_NAMES,
  CLAUDE_MODELS,
  type ClaudeModel,
  DEFAULT_CLAUDE_MODEL,
} from '@moatless/shared/constants'
import { useState } from 'react'
import { useBranches, useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { useTemplatesList } from '@/features/templates'

// Repository selector hook
export function useRepositorySelector(enabled = true, initialValue = '') {
  const [selected, setSelected] = useState(initialValue)
  const { data, isLoading } = useRepositoriesList({ enabled })

  const items =
    data?.map((repo) => ({
      id: repo.id,
      label: repo.name,
    })) || []

  return {
    value: selected,
    onChange: setSelected,
    loading: isLoading,
    placeholder: 'Select repository',
    items,
    width: 'w-[180px]',
  }
}

// Branch selector hook
export function useBranchSelector(repositoryId?: string, enabled = true) {
  const [selected, setSelected] = useState('')
  const { data, isLoading } = useBranches(repositoryId || '', {
    enabled: enabled && !!repositoryId,
  })

  const items =
    data?.branches.map((branch) => ({
      id: branch,
      label: branch,
    })) || []

  return {
    value: selected,
    onChange: setSelected,
    loading: isLoading,
    placeholder: 'Select branch',
    items,
    width: 'w-[140px]',
    disabled: !repositoryId,
  }
}

// Template selector hook
export function useTemplateSelector(enabled = true) {
  const [selected, setSelected] = useState('')
  const { data, isLoading } = useTemplatesList({ enabled })

  const items =
    data?.templates.map((template) => ({
      id: template.id,
      label: template.name,
    })) || []

  return {
    value: selected,
    onChange: setSelected,
    loading: isLoading,
    placeholder: 'Select template',
    items,
    width: 'w-[180px]',
  }
}

// Model selector hook
export function useModelSelector(initial: ClaudeModel = DEFAULT_CLAUDE_MODEL) {
  const [selected, setSelected] = useState<ClaudeModel>(initial)

  const items = [
    { id: CLAUDE_MODELS.OPUS_4, label: CLAUDE_MODEL_DISPLAY_NAMES[CLAUDE_MODELS.OPUS_4] },
    { id: CLAUDE_MODELS.SONNET_4, label: CLAUDE_MODEL_DISPLAY_NAMES[CLAUDE_MODELS.SONNET_4] },
  ]

  return {
    value: selected,
    onChange: setSelected as (value: string) => void,
    placeholder: 'Select model',
    items,
    width: 'w-[130px]',
  }
}
