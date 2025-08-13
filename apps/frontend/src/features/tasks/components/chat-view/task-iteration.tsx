import type { TaskIteration as TaskIterationType } from '@moatless/types'
import { AlertCircle } from 'lucide-react'
import type React from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { useSelectedIteration } from '@/features/tasks/contexts/task-ui-provider'
import { Alert, AlertDescription } from '@/lib/components/ui/alert'
import { TaskIterationStats } from './task-iteration-stats'

interface TaskIterationProps {
  iteration: TaskIterationType
  index: number
  messageCount: number
  isLastIteration: boolean
  status?: string
}

export function TaskIteration({
  iteration,
  index,
  messageCount,
  isLastIteration,
  status,
}: TaskIterationProps) {
  const { setSelectedIteration } = useSelectedIteration()
  const isRunningIteration = isLastIteration && status === 'running'

  const handleIterationClick = () => {
    setSelectedIteration(iteration)
  }

  const handleKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      handleIterationClick()
    }
  }

  const getIterationTiming = () => {
    const duration = iteration.endTime
      ? new Date(iteration.endTime).getTime() - new Date(iteration.startTime).getTime()
      : 0
    return {
      isRunning: isRunningIteration,
      duration,
      startTime:
        iteration.startTime instanceof Date
          ? iteration.startTime.toISOString()
          : iteration.startTime,
    }
  }

  const timing = getIterationTiming()

  return (
    <div key={iteration.iterationNumber || `iteration-${index}`} className="space-y-4">
      {/* Initial Message */}
      <div className="bg-secondary rounded-lg p-4">
        <div className="text-sm text-foreground prose prose-sm dark:prose-invert max-w-none">
          <ReactMarkdown remarkPlugins={[remarkGfm]}>{iteration.initialMessage}</ReactMarkdown>
        </div>
      </div>

      {/* Task Iteration Stats - Hide for running iteration */}
      {!isRunningIteration && (
        <TaskIterationStats
          timing={timing}
          messageCount={messageCount}
          cost={iteration.costUsd}
          willNotResume={iteration.willNotResume}
          onIterationClick={handleIterationClick}
          onKeyDown={handleKeyDown}
        />
      )}

      {/* Result Message or Error */}
      {iteration.status === 'failed' && iteration.result ? (
        <Alert variant="destructive">
          <AlertCircle className="h-4 w-4" />
          <AlertDescription>
            <div className="prose prose-sm dark:prose-invert max-w-none">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{iteration.result}</ReactMarkdown>
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
}
