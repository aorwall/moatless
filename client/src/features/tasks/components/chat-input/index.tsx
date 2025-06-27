import { useEffect, useState } from 'react'

import { useEnvironmentsList } from '@/features/environments/hooks/use-environments'
import { useStopAgent, useTask } from '@/features/tasks/hooks/use-tasks'
import { formatRunningDuration } from '@/features/tasks/utils/format-duration'

import { ChatForm } from './chat-form'
import { EnvironmentSelector } from './environment-selector'

interface ChatInputProps extends React.HTMLAttributes<HTMLDivElement> {
  // For task context
  taskId?: string
  placeholder?: string

  // For homepage context
  showEnvironmentSelector?: boolean
  showTitle?: boolean
  title?: string

  // Task status for conditional behavior
  taskStatus?: string

  // Callbacks
  onSendMessage?: (taskId: string, message: string, clearMessage: () => void) => void
  onCreateTask?: (message: string, environmentId: string, clearMessage: () => void) => void
  isLoading?: boolean
}

export function ChatInput({
  taskId,
  placeholder,
  taskStatus,
  showEnvironmentSelector = false,
  showTitle = false,
  title = 'What should we build today?',
  onSendMessage,
  onCreateTask,
  isLoading = false,
  className = '',
  ...divProps
}: ChatInputProps) {
  const [message, setMessage] = useState('')
  const [selectedEnvironmentId, setSelectedEnvironmentId] = useState<string | undefined>()
  const [runningDuration, setRunningDuration] = useState<string>('')

  // Get task data to access iteration start time
  const { data: task } = useTask(taskId || '', {
    enabled: !!taskId && taskStatus === 'running',
  })

  // Stop agent mutation
  const stopAgent = useStopAgent()

  // Environment data (only needed for homepage)
  const { data: environments, isLoading: environmentsLoading } = useEnvironmentsList({
    enabled: showEnvironmentSelector,
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

  const handleSendMessage = () => {
    // If running, stop the agent instead of sending message
    if (isRunning && taskId) {
      stopAgent.mutate(taskId)
      return
    }

    if (!message.trim()) {
      return
    }

    const clearMessage = () => setMessage('')

    if (isTaskContext) {
      // Task context - send message
      if (onSendMessage && !isPending) {
        onSendMessage(taskId, message.trim(), clearMessage)
      }
    } else if (selectedEnvironmentId && onCreateTask && !isPending) {
      // Homepage context - create task
      onCreateTask(message.trim(), selectedEnvironmentId, clearMessage)
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
    !(isTaskContext || selectedEnvironmentId)

  return (
    <div className={className} {...divProps}>
      {showTitle && (
        <div className="text-center mb-4">
          <h1 className="text-4xl font-bold tracking-tight">{title}</h1>
        </div>
      )}

      <div className={showTitle ? 'max-w-2xl mx-auto space-y-4' : 'space-y-4'}>
        <ChatForm
          message={message}
          onMessageChange={setMessage}
          sendMessage={handleSendMessage}
          textareaDisabled={isTextareaDisabled}
          sendDisabled={isSendDisabled}
          placeholder={getPlaceholderText()}
          environmentId={selectedEnvironmentId}
          taskId={taskId}
          isTaskContext={isTaskContext}
          buttonText={getButtonText()}
        />

        {showEnvironmentSelector && (
          <div className="flex flex-col sm:flex-row gap-4">
            <EnvironmentSelector
              environments={environments}
              selectedEnvironmentId={selectedEnvironmentId}
              onEnvironmentChange={setSelectedEnvironmentId}
              disabled={isPending}
              loading={environmentsLoading}
            />
          </div>
        )}
      </div>
    </div>
  )
}
