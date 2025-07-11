import type { Command } from '@moatless/shared/types/command'
import { EventType } from '@moatless/shared/types/event'
import type { Task, TaskIteration } from '@moatless/shared/types/task'
import { useParams } from '@tanstack/react-router'
import { AlertCircle, ArrowDown, Clock, DollarSign, InfoIcon, MessageSquare } from 'lucide-react'
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { toast } from 'sonner'
import {
  useEventStream,
  useStreamingStatus,
  useTaskData,
  useWorkspaceStatus,
} from '@/features/tasks/contexts/task-providers'
import { useSelectedIteration } from '@/features/tasks/contexts/task-ui-provider'
import { useAutoScroll } from '@/features/tasks/hooks/use-auto-scroll'
import { useSendMessage, useTaskCommands } from '@/features/tasks/hooks/use-tasks'
import { ErrorBoundary } from '@/lib/components/error-boundary'
import { Alert, AlertDescription } from '@/lib/components/ui/alert'
import { Button } from '@/lib/components/ui/button'
import { IsolatedTimer } from '@/lib/components/ui/isolated-timer'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/lib/components/ui/tooltip'
import { getErrorMessage } from '@/lib/utils/error'
import { ChatInput } from '../chat-input'
import { TaskMinifiedLastEventsDisplay } from '../event-viewer'
import { ChatHeader } from './chat-header'
import { ChatViewSkeleton } from './chat-view-skeleton'
import { CommandButtons } from './command-buttons'

function ChatViewComponent() {
  const params = useParams({ from: '/tasks/$taskId' })

  // Use specialized hooks from tri-context pattern
  const { taskId, status, iterations, repositoryId, isLoading: isLoadingTask } = useTaskData()
  const { events } = useEventStream()
  const { isStreaming } = useStreamingStatus()
  const { setSelectedIteration } = useSelectedIteration()
  const { scrollRef } = useAutoScroll()
  const bottomRef = useRef<HTMLDivElement>(null)

  // Derive isAtBottom state from scroll position
  const [scrollTop, setScrollTop] = useState(0)
  const [scrollHeight, setScrollHeight] = useState(0)
  const [clientHeight, setClientHeight] = useState(0)

  // Calculate if at bottom based on scroll values
  const isAtBottom = useMemo(() => {
    return scrollHeight - scrollTop - clientHeight < 50
  }, [scrollTop, scrollHeight, clientHeight])

  // Handle scroll to bottom button click
  const scrollToBottom = useCallback(() => {
    if (bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  }, [])

  // Get workspace status from context (doesn't block loading) - no longer needed for command filtering
  useWorkspaceStatus()

  // Fetch available commands for this task
  const { data: commandsData } = useTaskCommands(params.taskId, {
    enabled: !!params.taskId && status !== 'running',
  })

  // Task message sending hook
  const sendMessage = useSendMessage(params.taskId, {
    onError: (error) => {
      console.error('Failed to send message:', error)
      toast.error('Failed to send message', {
        description: getErrorMessage(error),
      })
    },
  })

  // Handle sending message to task - wrapped in useCallback for stable reference
  const handleSendMessage = useCallback(
    (
      _taskId: string,
      message: string,
      clearMessage: () => void,
      resume?: boolean,
      model?: string,
      filePaths?: string[],
      commandName?: string,
    ) => {
      sendMessage.mutate(
        { message, resume, model, filePaths, commandName },
        {
          onSuccess: () => {
            clearMessage()
          },
        },
      )
    },
    [sendMessage],
  )

  // Throttled scroll handler to update scroll values
  useEffect(() => {
    const scrollArea = scrollRef.current
    if (!scrollArea) return

    const viewport = scrollArea.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
    if (!viewport) return

    let rafId: number | null = null

    const handleScroll = () => {
      if (rafId) return

      rafId = requestAnimationFrame(() => {
        setScrollTop(viewport.scrollTop)
        setScrollHeight(viewport.scrollHeight)
        setClientHeight(viewport.clientHeight)
        rafId = null
      })
    }

    // Initial values
    setScrollTop(viewport.scrollTop)
    setScrollHeight(viewport.scrollHeight)
    setClientHeight(viewport.clientHeight)

    viewport.addEventListener('scroll', handleScroll)

    // Also observe for size changes
    const resizeObserver = new ResizeObserver(() => {
      setScrollHeight(viewport.scrollHeight)
      setClientHeight(viewport.clientHeight)
    })
    resizeObserver.observe(viewport)

    return () => {
      viewport.removeEventListener('scroll', handleScroll)
      resizeObserver.disconnect()
      if (rafId) cancelAnimationFrame(rafId)
    }
  }, [scrollRef])

  useEffect(() => {
    // Scroll to bottom when component mounts (initial load)
    if (bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  }, []) // Only on mount

  useEffect(() => {
    // Scroll to bottom when user is already at bottom and content changes
    if (bottomRef.current && isAtBottom) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' })
    }
  }, [isAtBottom])

  // Get last iteration for ChatInput
  const lastIteration =
    iterations && iterations.length > 0 ? iterations[iterations.length - 1] : null

  // Memoize message counts calculation for better performance
  const messageCounts = useMemo(() => {
    const counts = new Map<number, number>()

    if (isStreaming && events.length) {
      // Count message events with role assistant for each iteration
      events.forEach((event) => {
        if (
          event.type === EventType.MESSAGE &&
          event.payload?.role === 'assistant' &&
          event.iterationId
        ) {
          const currentCount = counts.get(event.iterationId) || 0
          counts.set(event.iterationId, currentCount + 1)
        }
      })
    }

    return counts
  }, [events, isStreaming])

  // Get available commands from the API response - must be before any early returns
  const availableCommands = useMemo(() => {
    if (!commandsData || status === 'running') return []
    return commandsData.commands
  }, [commandsData, status])

  // Handle command click - must be before any early returns
  const handleCommandClick = useCallback(
    (command: Command) => {
      handleSendMessage(
        params.taskId,
        command.prompt,
        () => {},
        true,
        undefined,
        undefined,
        command.name,
      )
    },
    [params.taskId, handleSendMessage],
  )

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
          className="p-4 border-t border-border"
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

    const iterationId = iterationIndex + 1
    const dynamicCount = messageCounts.get(iterationId) || 0
    return dynamicCount || staticCount
  }

  // Get iteration timing info
  const getIterationTiming = (iteration: TaskIteration, index: number) => {
    const isLastIteration = index === (iterations?.length || 0) - 1
    const isRunning = isLastIteration && status === 'running'

    return {
      isRunning,
      duration: iteration.durationMs || 0,
      startTime: iteration.startTime,
    }
  }

  const isTaskRunning = status === 'running'

  // Check if last iteration has an error
  const lastIterationHasError =
    lastIteration && (lastIteration.status === 'error' || lastIteration.status === 'failed')

  return (
    <div className="h-full flex flex-col">
      <ChatHeader />
      <div className="flex-1 overflow-hidden relative">
        <ScrollArea className="h-full w-full" ref={scrollRef}>
          <div className="p-3 sm:p-4 lg:p-6 space-y-4 max-w-[48rem] mx-auto">
            {iterations?.map((iteration: TaskIteration, index: number) => {
              const timing = getIterationTiming(iteration, index)
              const messageCount = getMessageCountForIteration(index, iteration.messageCount)
              const isLastIteration = index === (iterations?.length || 0) - 1
              const isRunningIteration = isLastIteration && status === 'running'

              return (
                <div key={iteration.iterationNumber || `iteration-${index}`} className="space-y-4">
                  {/* Initial Message */}
                  <div className="bg-secondary rounded-lg p-4">
                    <div className="text-sm text-foreground prose prose-sm dark:prose-invert max-w-none">
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
                        className="cursor-pointer text-sm text-muted-foreground hover:text-foreground transition-colors flex items-center gap-4 bg-transparent border-none p-0"
                        onClick={() => handleIterationClick(iteration)}
                        onKeyDown={(e) => handleKeyDown(e, iteration)}
                      >
                        <div className="flex items-center gap-1">
                          <Clock className="h-4 w-4" />
                          {timing.isRunning ? (
                            <IsolatedTimer startTime={timing.startTime} />
                          ) : (
                            <span>{formatDuration(timing.duration)}</span>
                          )}
                        </div>
                        <div className="flex items-center gap-1">
                          <MessageSquare className="h-4 w-4" />
                          <span>{messageCount} messages</span>
                        </div>
                        <div className="flex items-center gap-1">
                          <DollarSign className="h-4 w-4" />
                          <span>{formatCost(iteration.costUsd)}</span>
                        </div>
                        {iteration.willNotResume && (
                          <TooltipProvider>
                            <Tooltip>
                              <TooltipTrigger asChild>
                                <div className="flex items-center gap-1 text-status-warning">
                                  <InfoIcon className="h-4 w-4" />
                                  <span className="text-xs">Won't resume</span>
                                </div>
                              </TooltipTrigger>
                              <TooltipContent>
                                <p>This iteration reached the maximum turn limit.</p>
                                <p>Its result will be prepended to the next message.</p>
                              </TooltipContent>
                            </Tooltip>
                          </TooltipProvider>
                        )}
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
                              <pre className="mt-2 text-xs overflow-x-auto whitespace-pre-wrap bg-muted p-2 rounded">
                                {iteration.error.stack}
                              </pre>
                            </details>
                          )}
                        </div>
                      </AlertDescription>
                    </Alert>
                  ) : iteration.result ? (
                    <div className="text-sm text-foreground prose prose-sm dark:prose-invert max-w-none">
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>{iteration.result}</ReactMarkdown>
                    </div>
                  ) : null}
                </div>
              )
            })}

            {/* Show last three events when task is running */}
            {isTaskRunning && (
              <div className="mb-4">
                <TaskMinifiedLastEventsDisplay />
              </div>
            )}

            {/* Show available command buttons */}
            {!isTaskRunning && (
              <CommandButtons
                commands={availableCommands as Command[]}
                onCommandClick={handleCommandClick}
                disabled={sendMessage.isPending}
              />
            )}

            <div ref={bottomRef} />
          </div>
        </ScrollArea>

        {/* Scroll to bottom indicator */}
        {!isAtBottom && (
          <div className="absolute bottom-4 right-4 z-10">
            <Button
              variant="outline"
              size="sm"
              onClick={scrollToBottom}
              className="shadow-lg bg-background/80 backdrop-blur-sm border-border/50 hover:bg-background/90 flex items-center gap-2"
            >
              <ArrowDown className="h-4 w-4" />
              Scroll to bottom
            </Button>
          </div>
        )}
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
          task={iterations ? ({ iterations, repositoryId } as unknown as Task) : undefined}
          taskStatus={status}
          lastIterationHasError={lastIterationHasError || false}
          onSendMessage={handleSendMessage}
          isLoading={sendMessage.isPending}
          className="p-4 border-t border-border"
          maxHeight="150px"
        />
      </ErrorBoundary>
    </div>
  )
}

export const ChatView = React.memo(ChatViewComponent)
