import { CLAUDE_MODELS, type ClaudeModel, DEFAULT_CLAUDE_MODEL } from '@moatless/shared/constants'
import type { UploadFileResponse } from '@moatless/shared/types/api'
import type { Task } from '@moatless/shared/types/task'
import { useEffect, useState } from 'react'
import { useRepositoriesList } from '@/features/repositories/hooks/use-repositories'
import { useStopAgent } from '@/features/tasks/hooks/use-tasks'
import { formatRunningDuration } from '@/features/tasks/utils/format-duration'

import { ChatForm } from './chat-form'

interface ChatInputProps extends React.HTMLAttributes<HTMLDivElement> {
  // For task context
  taskId?: string
  placeholder?: string
  task?: Task // Optional task data to avoid duplicate API calls

  // For homepage context
  showRepositorySelector?: boolean
  showTitle?: boolean
  title?: string

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
  ) => void
  isLoading?: boolean

  // Textarea configuration
  maxHeight?: string
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
  onSendMessage,
  onCreateTask,
  isLoading = false,
  className = '',
  maxHeight = '200px',
  ...divProps
}: ChatInputProps) {
  const [message, setMessage] = useState('')
  const [uploadedFiles, setUploadedFiles] = useState<UploadFileResponse[]>([])
  const [selectedRepositoryId, setSelectedRepositoryId] = useState<string | undefined>(() => {
    // Load last selected repository from localStorage
    if (typeof window !== 'undefined' && !taskId) {
      return localStorage.getItem('lastSelectedRepositoryId') || undefined
    }
    return undefined
  })
  const [runningDuration, setRunningDuration] = useState<string>('')
  const [resumeConversation, setResumeConversation] = useState(true)
  const [selectedModel, setSelectedModel] = useState<ClaudeModel>(() => {
    // Load last selected model from localStorage
    if (typeof window !== 'undefined') {
      const savedModel = localStorage.getItem('lastSelectedModel') as ClaudeModel
      return savedModel && Object.values(CLAUDE_MODELS).includes(savedModel)
        ? savedModel
        : DEFAULT_CLAUDE_MODEL
    }
    return DEFAULT_CLAUDE_MODEL
  })

  // Stop agent mutation
  const stopAgent = useStopAgent()

  // Repository data (only needed for homepage)
  const { data: repositories, isLoading: repositoriesLoading } = useRepositoriesList({
    enabled: showRepositorySelector,
  })

  const isTaskContext = !!taskId
  const isPending = isLoading
  const isRunning = taskStatus === 'running'

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

  // Save selected repository to localStorage
  useEffect(() => {
    if (selectedRepositoryId && !taskId) {
      localStorage.setItem('lastSelectedRepositoryId', selectedRepositoryId)
    }
  }, [selectedRepositoryId, taskId])

  // Save selected model to localStorage
  useEffect(() => {
    localStorage.setItem('lastSelectedModel', selectedModel)
  }, [selectedModel])

  const handleSendMessage = () => {
    // If running, stop the agent instead of sending message
    if (isRunning && taskId) {
      stopAgent.mutate(taskId)
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
          selectedModel,
          filePaths,
        )
      }
    } else if (selectedRepositoryId && onCreateTask && !isPending) {
      // Homepage context - create task with uploaded files
      onCreateTask(message.trim(), selectedRepositoryId, clearMessage, selectedModel, uploadedFiles)
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
    return 'Describe your coding task or feature... (use @ to reference files)'
  }

  // Always allow editing the textarea
  const isTextareaDisabled = false

  // But disable sending based on conditions
  // Allow clicking when running (to stop), but disable for other states
  const isSendDisabled =
    isPending ||
    (!isRunning && !message.trim()) ||
    (isTaskContext && (taskStatus === 'completed' || taskStatus === 'failed')) ||
    !(isTaskContext || selectedRepositoryId)

  return (
    <div className={className} {...divProps}>
      {showTitle && (
        <div className="text-center mb-4">
          <h1 className="text-4xl font-bold tracking-tight">{title}</h1>
        </div>
      )}

      <div className={showTitle ? 'max-w-2xl mx-auto space-y-4' : 'space-y-4'}>
        {isTaskContext && lastIterationHasError && (
          <div className="px-4 py-3 bg-orange-50 dark:bg-orange-900/20 border border-orange-200 dark:border-orange-800 rounded-lg">
            <div className="flex items-start gap-3">
              <div className="flex-1">
                <p className="text-sm text-orange-800 dark:text-orange-200 font-medium">
                  Previous iteration failed
                </p>
                <p className="text-sm text-orange-700 dark:text-orange-300 mt-1">
                  You can choose to start fresh without context from the previous conversation.
                </p>
              </div>
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={resumeConversation}
                  onChange={(e) => setResumeConversation(e.target.checked)}
                  className="rounded border-orange-300 text-orange-600 focus:ring-orange-500"
                />
                <span className="text-sm text-orange-700 dark:text-orange-300 whitespace-nowrap">
                  Resume conversation
                </span>
              </label>
            </div>
          </div>
        )}
        <ChatForm
          message={message}
          onMessageChange={setMessage}
          sendMessage={handleSendMessage}
          textareaDisabled={isTextareaDisabled}
          sendDisabled={isSendDisabled}
          placeholder={getPlaceholderText()}
          repositoryId={selectedRepositoryId}
          taskId={taskId}
          isTaskContext={isTaskContext}
          buttonText={getButtonText()}
          selectedModel={selectedModel}
          onModelChange={setSelectedModel}
          isRunning={isRunning}
          // Repository selector props
          showRepositorySelector={showRepositorySelector}
          repositories={repositories}
          selectedRepositoryId={selectedRepositoryId}
          onRepositoryChange={setSelectedRepositoryId}
          repositoriesLoading={repositoriesLoading}
          maxHeight={maxHeight}
          // File upload props
          uploadedFiles={uploadedFiles}
          onFilesChange={setUploadedFiles}
        />
      </div>
    </div>
  )
}
