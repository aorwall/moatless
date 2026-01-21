import { CheckCircle, FileText, Loader2, XCircle } from 'lucide-react'
import { useCallback } from 'react'
import { Response } from '@/lib/components/ai-elements/response'
import { Button } from '@/lib/components/ui/button'
import { cn } from '@/lib/utils'

interface PlanApprovalInputProps {
  plan?: string
  onApprove: () => void
  onReject: () => void
  isPending?: boolean
  error?: Error | null
}

export function PlanApprovalInput({
  plan,
  onApprove,
  onReject,
  isPending = false,
  error,
}: PlanApprovalInputProps) {
  const handleRejectClick = useCallback(() => {
    onReject()
  }, [onReject])

  return (
    <div className="z-10 flex-shrink-0 bg-background absolute bottom-0 left-0 right-0">
      <div data-testid="plan-approval-input" className="p-4">
        <div className="group flex flex-col gap-3 rounded-3xl border border-purple-500 bg-card p-4 relative shadow-sm">
          {/* Header */}
          <div className="flex items-center gap-2 text-sm font-medium text-purple-600 dark:text-purple-400">
            <FileText className="h-4 w-4" />
            <span>Plan Ready for Review</span>
          </div>

          {/* Plan content */}
          {plan && (
            <div
              className={cn(
                'max-h-[40vh] overflow-y-auto rounded-md bg-muted/50 p-3',
                'scrollbar-thin scrollbar-track-transparent scrollbar-thumb-muted',
              )}
            >
              <Response>{plan}</Response>
            </div>
          )}

          {/* Error message */}
          {error && (
            <p className="text-sm text-destructive">{error.message || 'Failed to submit'}</p>
          )}

          {/* Action buttons */}
          <div className="flex gap-2">
            <Button onClick={onApprove} disabled={isPending} className="flex-1 gap-2" size="sm">
              {isPending ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <CheckCircle className="h-4 w-4" />
              )}
              Approve Plan
            </Button>
            <Button
              onClick={handleRejectClick}
              disabled={isPending}
              variant="outline"
              className="flex-1 gap-2"
              size="sm"
            >
              {isPending ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <XCircle className="h-4 w-4" />
              )}
              Reject
            </Button>
          </div>
        </div>
      </div>
    </div>
  )
}
