import { CLAUDE_MODELS, type ClaudeModel, DEFAULT_CLAUDE_MODEL } from '@moatless/shared/constants'
import type { UploadFileResponse } from '@moatless/shared/types/api'
import type { Task } from '@moatless/shared/types/task'
import { useEffect, useState } from 'react'
import { useRepositoriesList, useBranches } from '@/features/repositories/hooks/use-repositories'
import { useStopAgent } from '@/features/tasks/hooks/use-tasks'
import { useTemplatesList } from '@/features/templates'
import { formatRunningDuration } from '@/features/tasks/utils/format-duration'

import { ChatForm } from './chat-form'

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
    repositoryName: string,
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
  mode = 'code',
  onSendMessage,
  onCreateTask,
  onCreateVibeTask,
  isLoading = false,
  className = '',
  maxHeight = '200px',
  ...divProps
}: ChatInputProps) {
  const [message, setMessage] = useState('')
  const [uploadedFiles, setUploadedFiles] = useState<UploadFileResponse[]>([])
  const [selectedRepositoryId, setSelectedRepositoryId] = useState<string | undefined>(() => {
    // Load last selected repository from localStorage
    if (typeof window !== 'undefined' && !taskId && mode === 'code') {
      return localStorage.getItem('lastSelectedRepositoryId') || undefined
    }
    return undefined
  })
  const [selectedTemplateId, setSelectedTemplateId] = useState<string | undefined>(() => {
    // Load last selected template from localStorage
    if (typeof window !== 'undefined' && !taskId && mode === 'vibe') {
      return localStorage.getItem('lastSelectedTemplateId') || undefined
    }
    return undefined
  })
  const [repositoryName, setRepositoryName] = useState<string>('')
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
  const [selectedBranch, setSelectedBranch] = useState<string | undefined>(undefined)

  // Stop agent mutation - only initialize if we have a taskId
  const stopAgent = useStopAgent(taskId || '')

  // Repository data (only needed for homepage in code mode)
  const { data: repositories, isLoading: repositoriesLoading } = useRepositoriesList({
    enabled: showRepositorySelector && mode === 'code',
  })

  // Branch data (only needed for homepage when repository is selected in code mode)
  const { data: branchesData, isLoading: branchesLoading } = useBranches(
    selectedRepositoryId || '',
    {
      enabled: showRepositorySelector && mode === 'code' && !!selectedRepositoryId,
    }
  )
  const branches = branchesData?.branches || []

  // Template data (only needed for homepage in vibe mode)
  const { data: templatesData, isLoading: templatesLoading } = useTemplatesList({
    enabled: showRepositorySelector && mode === 'vibe',
  })
  const templates = templatesData?.templates || []

  const isTaskContext = !!taskId
  const isPending = isLoading
  const isRunning = taskStatus === 'running'
  
  // Extract repositoryId from task if available
  const effectiveRepositoryId = isTaskContext ? task?.repositoryId : selectedRepositoryId

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
    if (selectedRepositoryId && !taskId && mode === 'code') {
      localStorage.setItem('lastSelectedRepositoryId', selectedRepositoryId)
    }
  }, [selectedRepositoryId, taskId, mode])

  // Save selected template to localStorage
  useEffect(() => {
    if (selectedTemplateId && !taskId && mode === 'vibe') {
      localStorage.setItem('lastSelectedTemplateId', selectedTemplateId)
    }
  }, [selectedTemplateId, taskId, mode])

  // Save selected model to localStorage
  useEffect(() => {
    localStorage.setItem('lastSelectedModel', selectedModel)
  }, [selectedModel])

  // Auto-select first branch when branches are loaded or reset when repository changes
  useEffect(() => {
    if (showRepositorySelector && mode === 'code' && selectedRepositoryId) {
      if (branches.length > 0 && !selectedBranch) {
        setSelectedBranch(branches[0]) // Select the first branch (should be main/master)
      }
    } else {
      setSelectedBranch(undefined) // Reset branch when no repository
    }
  }, [showRepositorySelector, mode, selectedRepositoryId, branches, selectedBranch])

  // Auto-select first template when templates are loaded
  useEffect(() => {
    if (showRepositorySelector && mode === 'vibe' && templates.length > 0 && !selectedTemplateId) {
      setSelectedTemplateId(templates[0].id)
    }
  }, [showRepositorySelector, mode, templates, selectedTemplateId])

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
          selectedModel,
          filePaths,
        )
      }
    } else if (mode === 'code' && selectedRepositoryId && onCreateTask && !isPending) {
      // Code mode - create task with uploaded files and selected branch
      onCreateTask(message.trim(), selectedRepositoryId, clearMessage, selectedModel, uploadedFiles, selectedBranch)
    } else if (mode === 'vibe' && selectedTemplateId && repositoryName.trim() && onCreateVibeTask && !isPending) {
      // Vibe mode - create vibe task with template
      onCreateVibeTask(message.trim(), selectedTemplateId, repositoryName.trim(), clearMessage, selectedModel, uploadedFiles)
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

  // Always allow editing the textarea
  const isTextareaDisabled = false

  // But disable sending based on conditions
  // Allow clicking when running (to stop), but disable for other states
  const isSendDisabled =
    isPending ||
    (!isRunning && !message.trim()) ||
    (isTaskContext && (taskStatus === 'completed' || taskStatus === 'failed')) ||
    !(isTaskContext || 
      (mode === 'code' && selectedRepositoryId) || 
      (mode === 'vibe' && selectedTemplateId && repositoryName.trim())
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
          sendMessage={handleSendMessage}
          textareaDisabled={isTextareaDisabled}
          sendDisabled={isSendDisabled}
          placeholder={getPlaceholderText()}
          repositoryId={effectiveRepositoryId}
          taskId={taskId}
          isTaskContext={isTaskContext}
          buttonText={getButtonText()}
          selectedModel={selectedModel}
          onModelChange={setSelectedModel}
          isRunning={isRunning}
          isLoading={isPending}
          mode={mode}
          // Repository selector props (code mode)
          showRepositorySelector={showRepositorySelector && mode === 'code'}
          repositories={repositories}
          selectedRepositoryId={selectedRepositoryId}
          onRepositoryChange={setSelectedRepositoryId}
          repositoriesLoading={repositoriesLoading}
          // Branch selector props (code mode)
          showBranchSelector={showRepositorySelector && mode === 'code'}
          branches={branches}
          selectedBranch={selectedBranch}
          onBranchChange={setSelectedBranch}
          branchesLoading={branchesLoading}
          // Template selector props (vibe mode)
          showTemplateSelector={showRepositorySelector && mode === 'vibe'}
          templates={templates}
          selectedTemplateId={selectedTemplateId}
          onTemplateChange={setSelectedTemplateId}
          templatesLoading={templatesLoading}
          // Repository name input (vibe mode)
          repositoryName={repositoryName}
          onRepositoryNameChange={setRepositoryName}
          maxHeight={maxHeight}
          // File upload props
          uploadedFiles={uploadedFiles}
          onFilesChange={setUploadedFiles}
        />
      </div>
    </div>
  )
}
