import { CheckCircle, ChevronDown, ChevronRight, FileText, XCircle } from 'lucide-react'
import { Response } from '@/lib/components/ai-elements/response'
import type { AllowedPrompt } from '@/lib/types/user-input'
import { cn } from '@/lib/utils'
import type { ToolCallProps } from './tool-call-props.ts'

/**
 * Plan-approval summary for the ExitPlanMode tool. Pending approvals (no
 * result yet) are rendered in the input area instead, so this returns `null`
 * for them.
 */
export function ExitPlanModeToolCall({
  content,
  toolResult,
  expanded,
  onToggle,
  hasPendingUserInput,
}: ToolCallProps) {
  // If pending (will be rendered in input area), skip rendering here
  if (hasPendingUserInput && !toolResult) {
    return null
  }

  const toolInput = content.toolInput as
    | { allowedPrompts?: AllowedPrompt[]; plan?: string }
    | undefined
  const hasPlan = !!toolInput?.plan
  const isExpandable = hasPlan
  const isResolved = !!toolResult
  const wasApproved = toolResult && !toolResult.isError

  return (
    <div className="text-xs font-mono w-full max-w-full">
      <div className="flex items-start gap-2">
        <div className="w-1.5 h-1.5 bg-primary rounded-full mt-1 flex-shrink-0" />
        <div className="flex-1 min-w-0">
          {/* Header */}
          {/* biome-ignore lint/a11y/noStaticElementInteractions: Role and keyboard handler are conditionally added when interactive */}
          <div
            role={isExpandable ? 'button' : undefined}
            tabIndex={isExpandable ? 0 : undefined}
            onClick={isExpandable ? onToggle : undefined}
            onKeyDown={
              isExpandable
                ? (e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault()
                      onToggle()
                    }
                  }
                : undefined
            }
            className={cn(
              'flex items-center gap-1.5 min-w-0',
              isExpandable && 'cursor-pointer hover:text-foreground transition-colors',
            )}
          >
            {isExpandable && (
              <span className="flex-shrink-0">
                {expanded ? (
                  <ChevronDown className="h-3 w-3" />
                ) : (
                  <ChevronRight className="h-3 w-3" />
                )}
              </span>
            )}
            <FileText className="h-3 w-3 text-primary flex-shrink-0" />
            <span className="font-medium flex-shrink-0">Plan approval</span>
            {isResolved ? (
              wasApproved ? (
                <>
                  <CheckCircle className="h-3 w-3 text-green-500 flex-shrink-0" />
                  <span className="text-green-600 dark:text-green-400">Approved</span>
                </>
              ) : (
                <>
                  <XCircle className="h-3 w-3 text-red-500 flex-shrink-0" />
                  <span className="text-red-600 dark:text-red-400">Rejected</span>
                </>
              )
            ) : (
              <span className="text-muted-foreground">Pending</span>
            )}
          </div>

          {/* Expanded plan content */}
          {isExpandable && expanded && (
            <div className="mt-2 bg-muted/30 p-3 rounded max-h-96 overflow-y-auto">
              <Response>{toolInput?.plan}</Response>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
