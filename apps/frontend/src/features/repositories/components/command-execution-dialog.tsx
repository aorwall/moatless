import type { CommandExecutionOptions } from '@moatless/types'
import { AlertCircle, Loader2, Play, StopCircle } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Terminal } from '@/lib/components/terminal/terminal'
import { Badge } from '@/lib/components/ui/badge'
import { Button } from '@/lib/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'
import { Progress } from '@/lib/components/ui/progress'
import { useCommandExecution } from '../hooks/use-command-execution'

interface CommandExecutionDialogProps {
  repositoryId: string
  commands: string[]
  title?: string
  description?: string
  open: boolean
  onOpenChange: (open: boolean) => void
  onComplete?: (success: boolean) => void
}

export function CommandExecutionDialog({
  repositoryId,
  commands,
  title = 'Executing Commands',
  description,
  open,
  onOpenChange,
  onComplete,
}: CommandExecutionDialogProps) {
  const {
    lines,
    isExecuting,
    error,
    currentCommandIndex,
    totalCommands,
    summary,
    executeCommands,
    stopExecution,
  } = useCommandExecution()

  const [hasStarted, setHasStarted] = useState(false)

  // Auto-start execution when dialog opens
  useEffect(() => {
    if (open && !hasStarted && commands.length > 0) {
      const options: CommandExecutionOptions = {
        commands,
        purpose: 'setup',
        workingDirectory: '.',
        continueOnError: true,
        timeout: 300000, // 5 minutes timeout
      }
      executeCommands(repositoryId, options)
      setHasStarted(true)
    }
  }, [open, hasStarted, commands, repositoryId, executeCommands])

  // Reset state when dialog closes
  useEffect(() => {
    if (!open) {
      setHasStarted(false)
    }
  }, [open])

  // Call onComplete when execution finishes
  useEffect(() => {
    if (summary && onComplete) {
      const success = summary.failed === 0
      onComplete(success)
    }
  }, [summary, onComplete])

  const handleClose = () => {
    if (isExecuting) {
      // If still executing, stop the execution first
      stopExecution()
    }
    onOpenChange(false)
  }

  // Calculate progress percentage
  const progressPercentage =
    currentCommandIndex !== null && totalCommands > 0
      ? ((currentCommandIndex + 1) / totalCommands) * 100
      : 0

  return (
    <Dialog
      open={open}
      onOpenChange={(newOpen) => {
        if (!newOpen && isExecuting) {
          // Stop execution if dialog is being closed while still running
          stopExecution()
        }
        onOpenChange(newOpen)
      }}
    >
      <DialogContent className="max-w-7xl max-h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description && <DialogDescription>{description}</DialogDescription>}
        </DialogHeader>

        <div className="flex-1 space-y-4 overflow-hidden">
          {/* Status Bar */}
          <div className="flex items-center justify-between gap-4">
            <div className="flex items-center gap-2">
              {isExecuting ? (
                <>
                  <Loader2 className="h-4 w-4 animate-spin" />
                  <span className="text-sm font-medium">
                    Running command {(currentCommandIndex ?? 0) + 1} of {totalCommands}
                  </span>
                </>
              ) : summary ? (
                <>
                  {summary.failed === 0 ? (
                    <Badge variant="default" className="gap-1">
                      <Play className="h-3 w-3" />
                      Completed
                    </Badge>
                  ) : (
                    <Badge variant="destructive" className="gap-1">
                      <AlertCircle className="h-3 w-3" />
                      Failed
                    </Badge>
                  )}
                  <span className="text-sm">
                    {summary.succeeded}/{summary.total} commands succeeded
                  </span>
                </>
              ) : error ? (
                <>
                  <Badge variant="destructive" className="gap-1">
                    <AlertCircle className="h-3 w-3" />
                    Error
                  </Badge>
                  <span className="text-sm text-destructive">{error}</span>
                </>
              ) : (
                <span className="text-sm text-muted-foreground">Initializing...</span>
              )}
            </div>

            {isExecuting && (
              <Button size="sm" variant="outline" onClick={stopExecution}>
                <StopCircle className="mr-2 h-4 w-4" />
                Stop
              </Button>
            )}
          </div>

          {/* Progress Bar */}
          {isExecuting && totalCommands > 0 && (
            <div className="space-y-2">
              <Progress value={progressPercentage} className="h-2" />
              <div className="flex justify-between text-xs text-muted-foreground">
                <span>Progress: {Math.round(progressPercentage)}%</span>
                <span>
                  {(currentCommandIndex ?? 0) + 1} / {totalCommands} commands
                </span>
              </div>
            </div>
          )}

          {/* Terminal Output */}
          <div className="flex-1 min-h-0">
            <Terminal lines={lines} className="h-full" />
          </div>

          {/* Summary */}
          {summary && (
            <div className="border-t pt-4">
              <div className="flex items-center justify-between">
                <div className="space-y-1">
                  <p className="text-sm font-medium">Execution Summary</p>
                  <div className="flex gap-4 text-sm text-muted-foreground">
                    <span>Total: {summary.total}</span>
                    <span className="text-green-600">Succeeded: {summary.succeeded}</span>
                    {summary.failed > 0 && (
                      <span className="text-red-600">Failed: {summary.failed}</span>
                    )}
                    <span>Duration: {summary.duration}ms</span>
                  </div>
                </div>
                <Button
                  onClick={handleClose}
                  variant={summary.failed === 0 ? 'default' : 'outline'}
                >
                  {summary.failed === 0 ? 'Done' : 'Close'}
                </Button>
              </div>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
