import { EventType } from '@shared/types/event'
import type { TaskIteration } from '@shared/types/task'
import { useParams } from '@tanstack/react-router'
import { Clock, DollarSign, MessageSquare } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { toast } from 'sonner'
import { useTaskEventsWithStreaming } from '@/features/tasks/hooks/use-task-events'
import { useSendMessage, useTask } from '@/features/tasks/hooks/use-tasks'
import { useTaskUIStore } from '@/features/tasks/stores/task-ui-store'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { getErrorMessage } from '@/lib/utils/error'
import { ChatInput } from '../chat-input'
import { TaskMinifiedLastThreeEventsDisplay } from '../event-viewer'
import { ChatHeader } from './chat-header'

export function ChatView() {
  const params = useParams({ from: '/tasks/$taskId' })
  const { data: metadata } = useTask(params.taskId)
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
  const checkIfAtBottom = useCallback(() => {
    const scrollArea = scrollAreaRef.current
    if (!scrollArea) return false

    const viewport = scrollArea.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
    if (!viewport) return false

    const { scrollTop, scrollHeight, clientHeight } = viewport
    // Consider "at bottom" if within 50px of the bottom
    return scrollHeight - scrollTop - clientHeight < 50
  }, [])

  // Handle scroll events to track if user is at bottom
  const handleScroll = useCallback(() => {
    setIsAtBottom(checkIfAtBottom())
  }, [checkIfAtBottom])

  useEffect(() => {
    const scrollArea = scrollAreaRef.current
    if (!scrollArea) return

    const viewport = scrollArea.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
    if (!viewport) return

    viewport.addEventListener('scroll', handleScroll)
    return () => viewport.removeEventListener('scroll', handleScroll)
  }, [handleScroll])

  useEffect(() => {
    // Scroll to bottom when component mounts (initial load)
    if (bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  }, []) // Only on mount

  useEffect(() => {
    // Only scroll to bottom when task is running AND user is already at bottom
    if (bottomRef.current && isAtBottom && metadata?.status === 'running') {
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
    const isRunning = isLastIteration && metadata.status === 'running'

    if (!isRunning) {
      return iteration.durationMs || 0
    }

    // Calculate elapsed time since iteration started
    const startTime = new Date(iteration.startTime).getTime()
    return currentTime - startTime
  }

  const isTaskRunning = metadata.status === 'running'

  // Handle sending message to task
  const handleSendMessage = (taskId: string, message: string, clearMessage: () => void) => {
    sendMessage.mutate(
      { taskId, message },
      {
        onSuccess: () => {
          clearMessage()
        },
      },
    )
  }

  return (
    <div className="h-full flex flex-col">
      <ChatHeader />
      <div className="flex-1 overflow-hidden">
        <ScrollArea className="h-full w-full" ref={scrollAreaRef}>
          <div className="p-3 sm:p-4 lg:p-6 space-y-4 max-w-[48rem] mx-auto">
            {metadata.iterations?.map((iteration, index) => {
              const duration = getRunningDuration(iteration, index)
              const messageCount = getMessageCountForIteration(index, iteration.messageCount)
              const isLastIteration = index === (metadata.iterations?.length || 0) - 1
              const isRunningIteration = isLastIteration && metadata.status === 'running'

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
                    <div className="bg-destructive/10 border border-destructive/20 rounded-lg p-4">
                      <div className="flex items-start gap-3">
                        <div className="text-destructive mt-0.5">
                          <svg
                            width="16"
                            height="16"
                            viewBox="0 0 16 16"
                            fill="none"
                            xmlns="http://www.w3.org/2000/svg"
                            aria-label="Error icon"
                          >
                            <title>Error</title>
                            <path
                              d="M8 1.5C4.41 1.5 1.5 4.41 1.5 8C1.5 11.59 4.41 14.5 8 14.5C11.59 14.5 14.5 11.59 14.5 8C14.5 4.41 11.59 1.5 8 1.5ZM8 13C5.24 13 3 10.76 3 8C3 5.24 5.24 3 8 3C10.76 3 13 5.24 13 8C13 10.76 10.76 13 8 13ZM7.25 5V9H8.75V5H7.25ZM7.25 10V11.5H8.75V10H7.25Z"
                              fill="currentColor"
                            />
                          </svg>
                        </div>
                        <div className="flex-1">
                          <div className="text-sm font-medium text-destructive mb-1">
                            Iteration Failed
                          </div>
                          <div className="text-sm text-gray-700 dark:text-gray-300">
                            {iteration.error.message}
                          </div>
                          {iteration.error.stack && (
                            <details className="mt-2">
                              <summary className="text-xs text-gray-500 dark:text-gray-400 cursor-pointer hover:underline">
                                Show stack trace
                              </summary>
                              <pre className="mt-1 text-xs text-gray-600 dark:text-gray-400 overflow-x-auto whitespace-pre-wrap">
                                {iteration.error.stack}
                              </pre>
                            </details>
                          )}
                        </div>
                      </div>
                    </div>
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
          taskStatus={metadata?.status}
          onSendMessage={handleSendMessage}
          isLoading={sendMessage.isPending}
          className="p-4 border-t border-slate-200 dark:border-slate-700"
        />
      </ErrorBoundary>
    </div>
  )
}
