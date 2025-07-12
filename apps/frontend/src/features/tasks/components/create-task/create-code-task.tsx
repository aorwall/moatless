import type { ClaudeModel } from '@moatless/shared/constants'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useCreateTask } from '@/features/tasks/hooks/use-tasks'
import { getErrorMessage } from '@/lib/utils/error'
import {
  GenericSelector,
  useBranchSelector,
  useModelSelector,
  useRepositorySelector,
} from '../chat-input/selectors'
import { MessageInput } from './message-input'

interface CreateCodeTaskProps {
  className?: string
  maxHeight?: string
  showTitle?: boolean
  title?: string
  defaultRepositoryId?: string
  defaultMessage?: string
}

export function CreateCodeTask({
  className = '',
  maxHeight = '300px',
  showTitle = false,
  title = 'What should we build today?',
  defaultRepositoryId,
  defaultMessage,
}: CreateCodeTaskProps) {
  const [message, setMessage] = useState(defaultMessage || '')

  // Initialize model selector with saved value
  const initialModel = (() => {
    if (typeof window !== 'undefined') {
      const savedModel = localStorage.getItem('lastSelectedModel') as ClaudeModel
      return savedModel || undefined
    }
    return undefined
  })()

  const modelSelector = useModelSelector(initialModel)

  // Determine initial repository value
  const initialRepositoryId = (() => {
    if (defaultRepositoryId) return defaultRepositoryId
    if (typeof window !== 'undefined') {
      return localStorage.getItem('lastSelectedRepositoryId') || ''
    }
    return ''
  })()

  const repoSelector = useRepositorySelector(true, initialRepositoryId)
  const branchSelector = useBranchSelector(repoSelector.value, true)

  const createTask = useCreateTask({
    onError: (error) => {
      toast.error('Failed to create task', {
        description: getErrorMessage(error),
      })
    },
  })

  // Save selected repository to localStorage
  useEffect(() => {
    if (repoSelector.value) {
      localStorage.setItem('lastSelectedRepositoryId', repoSelector.value)
    }
  }, [repoSelector.value])

  // Save selected model to localStorage
  useEffect(() => {
    if (modelSelector.value) {
      localStorage.setItem('lastSelectedModel', modelSelector.value)
    }
  }, [modelSelector.value])

  // Auto-select first branch when branches are loaded
  useEffect(() => {
    if (repoSelector.value && branchSelector.items.length > 0 && !branchSelector.value) {
      branchSelector.onChange(branchSelector.items[0].id)
    }
  }, [repoSelector.value, branchSelector.items, branchSelector.value, branchSelector.onChange])

  const handleSubmit = () => {
    if (!message.trim() || !repoSelector.value) {
      return
    }

    createTask.mutate(
      {
        agentType: 'claude-code',
        message: message.trim(),
        repositoryId: repoSelector.value,
        model: modelSelector.value,
        originalBranch: branchSelector.value,
      },
      {
        onSuccess: () => {
          setMessage('')
        },
      },
    )
  }

  const isSendDisabled = createTask.isPending || !message.trim() || !repoSelector.value

  return (
    <div className={className}>
      {showTitle && (
        <div className="text-center mb-4">
          <h1 className="text-4xl font-bold tracking-tight">{title}</h1>
        </div>
      )}

      <div className={showTitle ? 'max-w-2xl mx-auto' : ''}>
        <MessageInput
          value={message}
          onChange={setMessage}
          onSubmit={handleSubmit}
          disabled={isSendDisabled}
          placeholder="Describe your coding task or feature... (use @ to reference files)"
          buttonText={createTask.isPending ? 'Starting...' : 'Start Task'}
          loading={createTask.isPending}
          maxHeight={maxHeight}
          repositoryId={repoSelector.value}
          selectors={
            <>
              <GenericSelector {...repoSelector} />
              <GenericSelector {...branchSelector} />
              <GenericSelector {...modelSelector} />
            </>
          }
        />
      </div>
    </div>
  )
}
