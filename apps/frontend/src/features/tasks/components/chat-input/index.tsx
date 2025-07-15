import type { ClaudeModel, Task, UploadFileResponse } from '@moatless/types'
import { useEffect, useState } from 'react'
import {
  useChatMessage,
  useChatModel,
  useChatResumeConversation,
} from '@/features/tasks/contexts/task-ui-provider'
import { useStopAgent } from '@/features/tasks/hooks/use-tasks'

import { ChatForm } from './chat-form'
import { GenericSelector, useModelSelector } from './selectors'

interface ChatInputProps extends React.HTMLAttributes<HTMLDivElement> {
  // Required for task context
  taskId: string
  task?: Task // Optional task data to avoid duplicate API calls

  // Optional customization
  placeholder?: string
  maxHeight?: string

  // Task status for conditional behavior
  taskStatus?: string
  lastIterationHasError?: boolean

  // Callbacks
  onSendMessage: (
    taskId: string,
    message: string,
    clearMessage: () => void,
    resume?: boolean,
    model?: ClaudeModel,
    filePaths?: string[],
  ) => void
  isLoading?: boolean
}

export function ChatInput({
  taskId,
  placeholder,
  task,
  taskStatus,
  lastIterationHasError = false,
  onSendMessage,
  isLoading = false,
  className = '',
  maxHeight = '200px',
  ...divProps
}: ChatInputProps) {
  // Use TaskUI context for isolated task state
  const { message, setMessage, clearMessage } = useChatMessage()
  const { selectedModel, setSelectedModel } = useChatModel()
  const { resumeConversation, setResumeConversation } = useChatResumeConversation()

  // Local component state (not types between tasks)
  const [uploadedFiles, setUploadedFiles] = useState<UploadFileResponse[]>([])

  // No localStorage initialization - each task maintains its own isolated model selection

  const modelSelector = useModelSelector(selectedModel)

  // Stop agent mutation
  const stopAgent = useStopAgent(taskId)

  const isPending = isLoading
  const isRunning = taskStatus === 'running'

  // Extract repositoryId from task
  const repositoryId = task?.repositoryId

  // Update context when model selector changes (isolated per task, no localStorage sync)
  useEffect(() => {
    if (modelSelector.value && modelSelector.value !== selectedModel) {
      setSelectedModel(modelSelector.value as ClaudeModel)
    }
  }, [modelSelector.value, selectedModel, setSelectedModel])

  const handleSendMessage = () => {
    // If running, stop the agent instead of sending message
    if (isRunning) {
      stopAgent.mutate()
      return
    }

    if (!message.trim()) {
      return
    }

    // Send message with file paths
    if (!isPending) {
      const filePaths = uploadedFiles.map((f) => f.absolutePath)

      // Create a stable callback that clears both message and files
      const clearMessageAndFiles = () => {
        clearMessage()
        setUploadedFiles([])
      }

      onSendMessage(
        taskId,
        message.trim(),
        clearMessageAndFiles,
        resumeConversation,
        modelSelector.value as ClaudeModel,
        filePaths,
      )
    }
  }

  const getButtonText = () => {
    if (taskStatus === 'running') {
      return 'Running'
    }
    if (taskStatus === 'completed' || taskStatus === 'failed') {
      return 'Task Completed'
    }
    return isPending ? 'Sending...' : 'Send'
  }

  const getPlaceholderText = () => {
    if (placeholder) {
      return placeholder
    }
    if (taskStatus === 'running') {
      return 'Task is running... Type your next request'
    }
    if (taskStatus === 'completed' || taskStatus === 'failed') {
      return `Task is ${taskStatus}. Type a new request to continue`
    }
    return 'Type a new request... (use @ to reference files)'
  }

  // Disable sending based on conditions
  const isSendDisabled = isPending || (!isRunning && !message.trim())

  return (
    <div className={className} {...divProps}>
      <div className="space-y-4">
        {lastIterationHasError && (
          <div className="px-4 py-3 bg-status-warning/10 border border-status-warning/20 rounded-lg">
            <div className="flex items-start gap-3">
              <div className="flex-1">
                <p className="text-sm text-status-warning font-medium">Previous iteration failed</p>
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
          repositoryId={repositoryId}
          taskId={taskId}
          isTaskContext={true}
          buttonText={getButtonText()}
          running={isRunning}
          loading={isPending}
          maxHeight={maxHeight}
          files={uploadedFiles}
          onFilesChange={setUploadedFiles}
          selectors={<GenericSelector {...modelSelector} />}
        />
      </div>
    </div>
  )
}
