import { EventType } from '@shared/types/event'
import type { Task, TaskIteration } from '@shared/types/task'
import { useParams } from '@tanstack/react-router'
import { AlertCircle, Clock, DollarSign, MessageSquare } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { toast } from 'sonner'
import {
  useEventStream,
  useSelectedIteration,
  useStreamingStatus,
  useTaskData,
} from '@/features/tasks/contexts/task-providers'
import { useAutoScroll } from '@/features/tasks/hooks/use-auto-scroll'
import { useSendMessage } from '@/features/tasks/hooks/use-tasks'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { Alert, AlertDescription } from '@/lib/components/ui/alert'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { getErrorMessage } from '@/lib/utils/error'
import { ChatInput } from '../chat-input'
import { TaskMinifiedLastThreeEventsDisplay } from '../event-viewer'
import { ChatHeader } from './chat-header'
import { ChatViewSkeleton } from './chat-view-skeleton'

export function ChatView() {
  const params = useParams({ from: '/tasks/$taskId' })

  // Use specialized hooks from tri-context pattern
  const {
    taskId,
    status,
    agentType,
    environmentId,
    iterations,
    createdAt,
    isLoading: isLoadingTask,
  } = useTaskData()
  const { events } = useEventStream()
  const { isStreaming } = useStreamingStatus()
  const { setSelectedIteration } = useSelectedIteration()
  const { scrollRef } = useAutoScroll()

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

  // Handle sending message to task - define early to avoid hoisting issues
  const handleSendMessage = (
    taskId: string,
    message: string,
    clearMessage: () => void,
    resume?: boolean,
    model?: string,
    filePaths?: string[],
  ) => {
    sendMessage.mutate(
      { taskId, message, resume, model, filePaths },
      {
        onSuccess: () => {
          clearMessage()
        },
      },
    )
  }

  // Update current time every second for running clock
  useEffect(() => {
    const interval = setInterval(() => {
      setCurrentTime(Date.now())
    }, 1000)
    return () => clearInterval(interval)
  }, [])

  // Check if user is at the bottom of the scroll area
  const checkIfAtBottom = useCallback(() => {
    const scrollArea = scrollRef.current
    if (!scrollArea) return false

    const viewport = scrollArea.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
    if (!viewport) return false

    const { scrollTop, scrollHeight, clientHeight } = viewport
    // Consider "at bottom" if within 50px of the bottom
    return scrollHeight - scrollTop - clientHeight < 50
  }, [scrollRef])

  // Handle scroll events to track if user is at bottom
  const handleScroll = useCallback(() => {
    setIsAtBottom(checkIfAtBottom())
  }, [checkIfAtBottom])

  useEffect(() => {
    const scrollArea = scrollRef.current
    if (!scrollArea) return

    const viewport = scrollArea.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
    if (!viewport) return

    viewport.addEventListener('scroll', handleScroll)
    return () => viewport.removeEventListener('scroll', handleScroll)
  }, [handleScroll, scrollRef])

  useEffect(() => {
    // Scroll to bottom when component mounts (initial load)
    if (bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  }, []) // Only on mount

  useEffect(() => {
    // Only scroll to bottom when task is running AND user is already at bottom
    if (bottomRef.current && isAtBottom && status === 'running') {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  })

  // Get last iteration for ChatInput
  const lastIteration =
    iterations && iterations.length > 0 ? iterations[iterations.length - 1] : null

  // Show skeleton during initial load
  if (isLoadingTask || !taskId) {
    return <ChatViewSkeleton />
  }

  // Show minimal UI while events are loading for the first time
  const isLoadingEvents = isLoadingTask
  if (isLoadingEvents && !events.length && !iterations?.length) {
    return (
      <div className="h-full flex flex-col">
        <ChatHeader />
        <div className="flex-1 overflow-hidden p-4">
          <div className="bg-secondary rounded-lg p-4 max-w-[48rem] mx-auto">
            <div className="text-sm text-muted-foreground text-center">Loading task data...</div>
          </div>
        </div>
        <ChatInput
          taskId={params.taskId}
          taskStatus={status}
          onSendMessage={handleSendMessage}
          isLoading={true}
          className="p-4 border-t border-slate-200 dark:border-slate-700"
        />
      </div>
    )
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
    const isLastIteration = index === (iterations?.length || 0) - 1
    const isRunning = isLastIteration && status === 'running'

    if (!isRunning) {
      return iteration.durationMs || 0
    }

    // Calculate elapsed time since iteration started
    const startTime = new Date(iteration.startTime).getTime()
    return currentTime - startTime
  }

  const isTaskRunning = status === 'running'

  // Check if last iteration has an error
  const lastIterationHasError =
    lastIteration && (lastIteration.status === 'error' || lastIteration.status === 'failed')

  return (
    <div className="h-full flex flex-col">
      <ChatHeader />
      <div className="flex-1 overflow-hidden">
        <ScrollArea className="h-full w-full" ref={scrollRef}>
          <div className="p-3 sm:p-4 lg:p-6 space-y-4 max-w-[48rem] mx-auto">
            {iterations?.map((iteration: TaskIteration, index: number) => {
              const duration = getRunningDuration(iteration, index)
              const messageCount = getMessageCountForIteration(index, iteration.messageCount)
              const isLastIteration = index === (iterations?.length || 0) - 1
              const isRunningIteration = isLastIteration && status === 'running'

              return (
                <div key={iteration.iterationNumber || `iteration-${index}`} className="space-y-4">
                  {/* Initial Message */}
                  <div className="bg-secondary rounded-lg p-4">
                    <div className="text-sm text-gray-900 dark:text-gray-100 prose prose-sm dark:prose-invert max-w-none">
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>
                        {iteration.initialMessage}
                      </ReactMarkdown>
                    </div>
                  </div>

                  {/* Iteration Info - Hide for running iteration */}
                  {!isRunningIteration && (
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
                          <span>{formatCost(iteration.costUsd)}</span>
                        </div>
                        <span className="text-xs">• Click to view events</span>
                      </button>
                    </div>
                  )}

                  {/* Result Message or Error */}
                  {(iteration.status === 'failed' || iteration.status === 'error') &&
                  iteration.error ? (
                    <Alert variant="destructive">
                      <AlertCircle className="h-4 w-4" />
                      <AlertDescription>
                        <div className="space-y-3">
                          {/* Show result first if available */}
                          {iteration.result && (
                            <div className="prose prose-sm dark:prose-invert max-w-none">
                              <ReactMarkdown remarkPlugins={[remarkGfm]}>
                                {iteration.result}
                              </ReactMarkdown>
                            </div>
                          )}

                          {/* Error message */}
                          <div>Error: {iteration.error.message}</div>

                          {/* Expandable stack trace */}
                          {iteration.error.stack && (
                            <details className="mt-2">
                              <summary className="text-xs cursor-pointer hover:underline">
                                Show stack trace
                              </summary>
                              <pre className="mt-2 text-xs overflow-x-auto whitespace-pre-wrap bg-black/10 dark:bg-white/10 p-2 rounded">
                                {iteration.error.stack}
                              </pre>
                            </details>
                          )}
                        </div>
                      </AlertDescription>
                    </Alert>
                  ) : iteration.result ? (
                    <div className="text-sm text-gray-900 dark:text-gray-100 prose prose-sm dark:prose-invert max-w-none">
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>{iteration.result}</ReactMarkdown>
                    </div>
                  ) : null}
                </div>
              )
            })}

            {/* Show last three events when task is running */}
            {isTaskRunning && (
              <div className="mb-4">
                <TaskMinifiedLastThreeEventsDisplay taskId={params.taskId} className="max-w-full" />
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
          task={iterations ? ({ iterations } as Task) : undefined}
          taskStatus={status}
          lastIterationHasError={lastIterationHasError || false}
          onSendMessage={handleSendMessage}
          isLoading={sendMessage.isPending}
          className="p-4 border-t border-slate-200 dark:border-slate-700"
          maxHeight="150px"
        />
      </ErrorBoundary>
    </div>
  )
}
