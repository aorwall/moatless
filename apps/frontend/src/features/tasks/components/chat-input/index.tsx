import { CLAUDE_MODELS, type ClaudeModel } from '@moatless/shared/constants'
import type { UploadFileResponse } from '@moatless/shared/types/api'
import type { Task } from '@moatless/shared/types/task'
import { useEffect, useState } from 'react'
import { useStopAgent } from '@/features/tasks/hooks/use-tasks'
import { formatRunningDuration } from '@/features/tasks/utils/format-duration'

import { ChatForm } from './chat-form'
import {
  GenericSelector,
  useRepositorySelector,
  useBranchSelector,
  useTemplateSelector,
  useModelSelector
} from './selectors'

export type ChatInputMode = 'code' | 'vibe'

interface ChatInputProps extends React.HTMLAttributes<HTMLDivElement> {
  // For task context
  taskId?: string
  placeholder?: string
  task?: Task // Optional task data to avoid duplicate API calls

  // For homepage context
  showRepositorySelector?: boolean
  showTitle?: boolean
  title?: string
  mode?: ChatInputMode // New: determines if showing repositories or templates

  // Task status for conditional behavior
  taskStatus?: string
  lastIterationHasError?: boolean


  // Callbacks
  onSendMessage?: (
    taskId: string,
    message: string,
    clearMessage: () => void,
    resume?: boolean,
    model?: ClaudeModel,
    filePaths?: string[],
  ) => void
  onCreateTask?: (
    message: string,
    repositoryId: string,
    clearMessage: () => void,
    model?: ClaudeModel,
    files?: UploadFileResponse[],
    originalBranch?: string,
  ) => void
  onCreateVibeTask?: (
    message: string,
    templateId: string,
    clearMessage: () => void,
    model?: ClaudeModel,
    files?: UploadFileResponse[],
  ) => void
  isLoading?: boolean

  // Textarea configuration
  maxHeight?: string
  
  // Default values
  defaultRepositoryId?: string
  defaultMessage?: string
}

export function ChatInput({
  taskId,
  placeholder,
  task, // Use provided task data instead of fetching
  taskStatus,
  lastIterationHasError = false,
  showRepositorySelector = false,
  showTitle = false,
  title = 'What should we build today?',
  mode = 'code',
  onSendMessage,
  onCreateTask,
  onCreateVibeTask,
  isLoading = false,
  className = '',
  maxHeight = '200px',
  defaultRepositoryId,
  defaultMessage,
  ...divProps
}: ChatInputProps) {
  const [message, setMessage] = useState(defaultMessage || '')
  const [uploadedFiles, setUploadedFiles] = useState<UploadFileResponse[]>([])
  const [runningDuration, setRunningDuration] = useState<string>('')
  const [resumeConversation, setResumeConversation] = useState(true)
  
  // Initialize model selector with saved value
  const initialModel = (() => {
    if (typeof window !== 'undefined') {
      const savedModel = localStorage.getItem('lastSelectedModel') as ClaudeModel
      return savedModel && Object.values(CLAUDE_MODELS).includes(savedModel)
        ? savedModel
        : undefined
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
  
  const repoSelector = useRepositorySelector(showRepositorySelector && mode === 'code', initialRepositoryId)
  const branchSelector = useBranchSelector(repoSelector.value, showRepositorySelector && mode === 'code')
  const templateSelector = useTemplateSelector(showRepositorySelector && mode === 'vibe')

  // Stop agent mutation - only initialize if we have a taskId
  const stopAgent = useStopAgent(taskId || '')

  const isTaskContext = !!taskId
  const isPending = isLoading
  const isRunning = taskStatus === 'running'
  
  // Extract repositoryId from task if available
  const effectiveRepositoryId = isTaskContext ? task?.repositoryId : repoSelector.value

  // Update running duration every second
  useEffect(() => {
    if (!isRunning || !task?.iterations?.length) {
      setRunningDuration('')
      return
    }

    const lastIteration = task.iterations[task.iterations.length - 1]
    const startTime = new Date(lastIteration.startTime)

    const updateDuration = () => {
      setRunningDuration(formatRunningDuration(startTime))
    }

    updateDuration() // Initial update
    const interval = setInterval(updateDuration, 1000)

    return () => clearInterval(interval)
  }, [isRunning, task?.iterations])

  
  // Initialize template from localStorage if in vibe mode
  useEffect(() => {
    if (!taskId && mode === 'vibe' && typeof window !== 'undefined') {
      const savedTemplateId = localStorage.getItem('lastSelectedTemplateId')
      if (savedTemplateId && !templateSelector.value) {
        templateSelector.onChange(savedTemplateId)
      }
    }
  }, [taskId, mode])
  
  // Save selected repository to localStorage
  useEffect(() => {
    if (repoSelector.value && !taskId && mode === 'code') {
      localStorage.setItem('lastSelectedRepositoryId', repoSelector.value)
    }
  }, [repoSelector.value, taskId, mode])

  // Save selected template to localStorage
  useEffect(() => {
    if (templateSelector.value && !taskId && mode === 'vibe') {
      localStorage.setItem('lastSelectedTemplateId', templateSelector.value)
    }
  }, [templateSelector.value, taskId, mode])

  // Save selected model to localStorage
  useEffect(() => {
    if (modelSelector.value) {
      localStorage.setItem('lastSelectedModel', modelSelector.value)
    }
  }, [modelSelector.value])

  // Auto-select first branch when branches are loaded
  useEffect(() => {
    if (showRepositorySelector && mode === 'code' && repoSelector.value) {
      if (branchSelector.items.length > 0 && !branchSelector.value) {
        branchSelector.onChange(branchSelector.items[0].id) // Select the first branch
      }
    }
  }, [showRepositorySelector, mode, repoSelector.value, branchSelector.items])

  // Auto-select first template when templates are loaded
  useEffect(() => {
    if (showRepositorySelector && mode === 'vibe' && templateSelector.items.length > 0 && !templateSelector.value) {
      templateSelector.onChange(templateSelector.items[0].id)
    }
  }, [showRepositorySelector, mode, templateSelector.items])
  
  // Reset message and files when taskId changes
  useEffect(() => {
    if (taskId) {
      setMessage('')
      setUploadedFiles([])
    }
  }, [taskId])

  const handleSendMessage = () => {
    // If running, stop the agent instead of sending message
    if (isRunning && taskId) {
      stopAgent.mutate()
      return
    }

    if (!message.trim()) {
      return
    }

    const clearMessage = () => {
      setMessage('')
      setUploadedFiles([])
    }

    if (isTaskContext) {
      // Task context - send message with file paths
      if (onSendMessage && !isPending) {
        // Convert file paths to array of paths for sending
        const filePaths = uploadedFiles.map((f) => f.absolutePath)
        onSendMessage(
          taskId,
          message.trim(),
          clearMessage,
          resumeConversation,
          modelSelector.value as ClaudeModel,
          filePaths,
        )
      }
    } else if (mode === 'code' && repoSelector.value && onCreateTask && !isPending) {
      // Code mode - create task with uploaded files and selected branch
      onCreateTask(message.trim(), repoSelector.value, clearMessage, modelSelector.value as ClaudeModel, uploadedFiles, branchSelector.value)
    } else if (mode === 'vibe' && templateSelector.value && onCreateVibeTask && !isPending) {
      // Vibe mode - create vibe task with template
      onCreateVibeTask(message.trim(), templateSelector.value, clearMessage, modelSelector.value as ClaudeModel, uploadedFiles)
    }
  }

  const getButtonText = () => {
    if (isTaskContext) {
      if (taskStatus === 'running') {
        return runningDuration || 'Running'
      }
      if (taskStatus === 'completed' || taskStatus === 'failed') {
        return 'Task Completed'
      }
      return isPending ? 'Sending...' : 'Send'
    }
    return isPending ? 'Starting...' : 'Start Task'
  }

  const getPlaceholderText = () => {
    if (placeholder) {
      return placeholder
    }
    if (isTaskContext) {
      if (taskStatus === 'running') {
        return 'Task is running... Type your next request'
      }
      if (taskStatus === 'completed' || taskStatus === 'failed') {
        return `Task is ${taskStatus}. Type a new request to continue`
      }
      return 'Type a new request... (use @ to reference files)'
    }
    if (mode === 'vibe') {
      return 'Describe what you want to build from this template... (use @ to reference files)'
    }
    return 'Describe your coding task or feature... (use @ to reference files)'
  }

  // Disable sending based on conditions
  // Allow clicking when running (to stop), but disable for other states
  const isSendDisabled =
    isPending ||
    (!isRunning && !message.trim()) ||
    (isTaskContext && (taskStatus === 'completed' || taskStatus === 'failed')) ||
    !(isTaskContext || 
      (mode === 'code' && repoSelector.value) || 
      (mode === 'vibe' && templateSelector.value)
    )

  return (
    <div className={className} {...divProps}>
      {showTitle && (
        <div className="text-center mb-4">
          <h1 className="text-4xl font-bold tracking-tight">{title}</h1>
        </div>
      )}

      <div className={showTitle ? 'max-w-2xl mx-auto space-y-4' : 'space-y-4'}>
        {isTaskContext && lastIterationHasError && (
          <div className="px-4 py-3 bg-status-warning/10 border border-status-warning/20 rounded-lg">
            <div className="flex items-start gap-3">
              <div className="flex-1">
                <p className="text-sm text-status-warning font-medium">
                  Previous iteration failed
                </p>
                <p className="text-sm text-status-warning mt-1">
                  You can choose to start fresh without context from the previous conversation.
                </p>
              </div>
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={resumeConversation}
                  onChange={(e) => setResumeConversation(e.target.checked)}
                  className="rounded border-status-warning/30 text-status-warning focus:ring-status-warning/50"
                />
                <span className="text-sm text-status-warning whitespace-nowrap">
                  Resume conversation
                </span>
              </label>
            </div>
          </div>
        )}
        <ChatForm
          message={message}
          onMessageChange={setMessage}
          onSubmit={handleSendMessage}
          disabled={isSendDisabled}
          placeholder={getPlaceholderText()}
          repositoryId={effectiveRepositoryId}
          taskId={taskId}
          isTaskContext={isTaskContext}
          buttonText={getButtonText()}
          running={isRunning}
          loading={isPending}
          maxHeight={maxHeight}
          files={uploadedFiles}
          onFilesChange={setUploadedFiles}
          selectors={[
            ...(showRepositorySelector && mode === 'code' ? [
              <GenericSelector key="repo" {...repoSelector} />,
              <GenericSelector key="branch" {...branchSelector} />
            ] : []),
            ...(showRepositorySelector && mode === 'vibe' ? [
              <GenericSelector key="template" {...templateSelector} />
            ] : []),
            <GenericSelector key="model" {...modelSelector} />
          ]}
        />
      </div>
    </div>
  )
}
