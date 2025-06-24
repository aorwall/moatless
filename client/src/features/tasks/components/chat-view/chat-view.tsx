import { EventType } from '@shared/types/event'
import { useParams } from '@tanstack/react-router'
import { Clock, DollarSign, Loader2, MessageSquare } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { toast } from 'sonner'
import { useTaskEventsWithStreaming } from '@/features/tasks/hooks/use-task-events'
import { useSendMessage, useTask, useTaskWorkspaceStatus } from '@/features/tasks/hooks/use-tasks'
import { useTaskUIStore } from '@/features/tasks/stores/task-ui-store'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { getErrorMessage } from '@/lib/utils/error'
import type { TaskIteration } from '@/types/task'

import { ChatInput } from '../chat-input'

export function ChatView() {
  const params = useParams({ from: '/tasks/$taskId' })
  const { data: metadata } = useTask(params.taskId)
  const { data: workspaceStatus } = useTaskWorkspaceStatus(params.taskId)
  const { events, isStreaming } = useTaskEventsWithStreaming(params.taskId)
  const setSelectedIteration = useTaskUIStore((state) => state.setSelectedIteration)
  const scrollAreaRef = useRef<HTMLDivElement>(null)
  const bottomRef = useRef<HTMLDivElement>(null)
  const [currentTime, setCurrentTime] = useState(Date.now())
  const [isAtBottom, setIsAtBottom] = useState(true)

  // Task message sending hook
  const sendMessage = useSendMessage({
    onError: (error) => {
      console.error('Failed to send message:', error)
      toast.error('Failed to send message', {
        description: getErrorMessage(error),
      })
    },
  })

  // Update current time every second for running clock
  useEffect(() => {
    const interval = setInterval(() => {
      setCurrentTime(Date.now())
    }, 1000)
    return () => clearInterval(interval)
  }, [])

  // Check if user is at the bottom of the scroll area
  const checkIfAtBottom = () => {
    const scrollArea = scrollAreaRef.current
    if (!scrollArea) return false

    const viewport = scrollArea.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
    if (!viewport) return false

    const { scrollTop, scrollHeight, clientHeight } = viewport
    // Consider "at bottom" if within 50px of the bottom
    return scrollHeight - scrollTop - clientHeight < 50
  }

  // Handle scroll events to track if user is at bottom
  const handleScroll = () => {
    setIsAtBottom(checkIfAtBottom())
  }

  useEffect(() => {
    const scrollArea = scrollAreaRef.current
    if (!scrollArea) return

    const viewport = scrollArea.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
    if (!viewport) return

    viewport.addEventListener('scroll', handleScroll)
    return () => viewport.removeEventListener('scroll', handleScroll)
  }, [])

  useEffect(() => {
    // Scroll to bottom when component mounts (initial load)
    if (bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  }, []) // Only on mount

  useEffect(() => {
    // Only scroll to bottom when task is running/workspace is active AND user is already at bottom
    if (
      bottomRef.current &&
      isAtBottom &&
      (metadata?.is_running || workspaceStatus?.status === 'running')
    ) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  })

  if (!metadata) {
    return null
  }

  const formatDuration = (durationMs: number) => {
    const minutes = Math.floor(durationMs / 60000)
    const seconds = Math.floor((durationMs % 60000) / 1000)
    return `${minutes}m ${seconds}s`
  }

  const formatCost = (cost: number) => {
    return `$${cost.toFixed(4)}`
  }

  const handleIterationClick = (iteration: TaskIteration) => {
    setSelectedIteration(iteration)
  }

  const handleKeyDown = (event: React.KeyboardEvent, iteration: TaskIteration) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      handleIterationClick(iteration)
    }
  }

  // Calculate message count from events for active iterations
  const getMessageCountForIteration = (iterationIndex: number, staticCount: number): number => {
    if (!(isStreaming && events.length)) {
      return staticCount
    }

    // Count message events with role assistant for this iteration (using index + 1 as iteration number)
    const messageEvents = events.filter(
      (event) =>
        event.iterationId === iterationIndex + 1 &&
        event.type === EventType.MESSAGE &&
        event.payload?.role === 'assistant',
    )

    return messageEvents.length || staticCount
  }

  // Calculate running duration for active iteration
  const getRunningDuration = (iteration: TaskIteration, index: number): number => {
    const isLastIteration = index === (metadata.iterations?.length || 0) - 1
    const isRunning =
      isLastIteration && (metadata.is_running || workspaceStatus?.status === 'running')

    if (!isRunning) {
      return iteration.duration_ms
    }

    // Calculate elapsed time since iteration started
    const startTime = new Date(iteration.start_time).getTime()
    return currentTime - startTime
  }

  const isWorkspaceRunning =
    workspaceStatus?.status === 'running' || workspaceStatus?.status === 'setting_up'

  // Handle sending message to task
  const handleSendMessage = (taskId: string, message: string) => {
    sendMessage.mutate({ taskId, message })
  }

  return (
    <div className="h-full flex flex-col">
      <div className="flex-1 overflow-hidden">
        <ScrollArea className="h-full w-full" ref={scrollAreaRef}>
          <div className="p-3 sm:p-4 lg:p-6 space-y-4 max-w-[48rem] mx-auto">
            {metadata.iterations?.map((iteration, index) => {
              const duration = getRunningDuration(iteration, index)
              const messageCount = getMessageCountForIteration(index, iteration.message_count)

              return (
                <div key={iteration.iteration_id || `iteration-${index}`} className="space-y-4">
                  {/* Initial Message */}
                  <div className="bg-secondary rounded-lg p-4">
                    <div className="text-sm text-gray-900 dark:text-gray-100 prose prose-sm dark:prose-invert max-w-none">
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>
                        {iteration.initial_message}
                      </ReactMarkdown>
                    </div>
                  </div>

                  {/* Iteration Info */}
                  <div className="flex justify-center py-2">
                    <button
                      type="button"
                      className="cursor-pointer text-sm text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-200 transition-colors flex items-center gap-4 bg-transparent border-none p-0"
                      onClick={() => handleIterationClick(iteration)}
                      onKeyDown={(e) => handleKeyDown(e, iteration)}
                    >
                      <div className="flex items-center gap-1">
                        <Clock className="h-4 w-4" />
                        <span>{formatDuration(duration)}</span>
                      </div>
                      <div className="flex items-center gap-1">
                        <MessageSquare className="h-4 w-4" />
                        <span>{messageCount} messages</span>
                      </div>
                      <div className="flex items-center gap-1">
                        <DollarSign className="h-4 w-4" />
                        <span>{formatCost(iteration.cost_usd)}</span>
                      </div>
                      <span className="text-xs">• Click to view events</span>
                    </button>
                  </div>

                  {/* Result Message */}
                  {iteration.result && (
                    <div className="text-sm text-gray-900 dark:text-gray-100 prose prose-sm dark:prose-invert max-w-none">
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>{iteration.result}</ReactMarkdown>
                    </div>
                  )}

                  {/* Error Message */}
                  {iteration.error_message && (
                    <div className="text-sm text-red-900 dark:text-red-400 prose prose-sm dark:prose-invert max-w-none">
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>
                        {iteration.error_message}
                      </ReactMarkdown>
                    </div>
                  )}
                </div>
              )
            })}

            {/* Show running state when workspace is running or task is running */}
            {(metadata.is_running || isWorkspaceRunning) &&
              metadata.iterations &&
              metadata.iterations.length > 0 && (
                <div className="flex items-center justify-center py-4 space-x-2">
                  <Loader2 className="h-5 w-5 animate-spin text-blue-500" />
                  <span className="text-sm text-gray-600 dark:text-gray-400">Processing...</span>
                </div>
              )}

            <div ref={bottomRef} />
          </div>
        </ScrollArea>
      </div>
      <ErrorBoundary
        componentName="Chat Input"
        fallback={(_error, reset) => (
          <div className="p-4 border-t text-sm text-muted-foreground">
            Chat input unavailable
            <button type="button" onClick={reset} className="ml-2 text-primary hover:underline">
              Retry
            </button>
          </div>
        )}
      >
        <ChatInput
          data-testid="task-chat-input"
          taskId={params.taskId}
          taskStatus={metadata?.status}
          onSendMessage={handleSendMessage}
          isLoading={sendMessage.isPending}
          className="p-4 border-t border-slate-200 dark:border-slate-700"
        />
      </ErrorBoundary>
    </div>
  )
}
