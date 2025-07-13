import type { ClaudeModel } from '@moatless/types/constants'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useCreateVibeTask } from '@/features/templates'
import { getErrorMessage } from '@/lib/utils/error'
import { GenericSelector, useModelSelector, useTemplateSelector } from '../chat-input/selectors'
import { MessageInput } from './message-input'

interface CreateVibeTaskProps {
  className?: string
  maxHeight?: string
  showTitle?: boolean
  title?: string
  defaultMessage?: string
  onSuccess?: () => void
}

export function CreateVibeTask({
  className = '',
  maxHeight = '300px',
  showTitle = false,
  title = 'What should we vibe today?',
  defaultMessage,
  onSuccess,
}: CreateVibeTaskProps) {
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
  const templateSelector = useTemplateSelector(true)

  const createVibeTask = useCreateVibeTask({
    onError: (error) => {
      toast.error('Failed to create vibe task', {
        description: getErrorMessage(error),
      })
    },
  })

  // Initialize template from localStorage
  useEffect(() => {
    if (typeof window !== 'undefined') {
      const savedTemplateId = localStorage.getItem('lastSelectedTemplateId')
      if (savedTemplateId && !templateSelector.value) {
        templateSelector.onChange(savedTemplateId)
      }
    }
  }, [templateSelector])

  // Save selected template to localStorage
  useEffect(() => {
    if (templateSelector.value) {
      localStorage.setItem('lastSelectedTemplateId', templateSelector.value)
    }
  }, [templateSelector.value])

  // Save selected model to localStorage
  useEffect(() => {
    if (modelSelector.value) {
      localStorage.setItem('lastSelectedModel', modelSelector.value)
    }
  }, [modelSelector.value])

  // Auto-select first template when templates are loaded
  useEffect(() => {
    if (templateSelector.items.length > 0 && !templateSelector.value) {
      templateSelector.onChange(templateSelector.items[0].id)
    }
  }, [templateSelector.items, templateSelector.value, templateSelector.onChange])

  const handleSubmit = () => {
    if (!message.trim() || !templateSelector.value) {
      return
    }

    // Generate a repository name based on template and timestamp
    const repositoryName = `${templateSelector.value}-${Date.now()}`

    createVibeTask.mutate(
      {
        agentType: 'claude-code',
        templateId: templateSelector.value,
        repositoryName,
        message: message.trim(),
        model: modelSelector.value,
        filePaths: [],
        start: true,
      },
      {
        onSuccess: () => {
          setMessage('')
          onSuccess?.()
        },
      },
    )
  }

  const isSendDisabled = createVibeTask.isPending || !message.trim() || !templateSelector.value

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
          placeholder="Describe what you want to build from this template..."
          buttonText={createVibeTask.isPending ? 'Starting...' : 'Start Task'}
          loading={createVibeTask.isPending}
          maxHeight={maxHeight}
          selectors={
            <>
              <GenericSelector {...templateSelector} />
              <GenericSelector {...modelSelector} />
            </>
          }
        />
      </div>
    </div>
  )
}
