import { CheckCircle, FileText, Loader2, X, XCircle } from 'lucide-react'
import { useState } from 'react'
import { Response } from '@/lib/components/ai-elements/response'
import { Button } from '@/lib/components/ui/button'
import { Textarea } from '@/lib/components/ui/textarea'
import { cn } from '@/lib/utils'

interface PlanApprovalInputProps {
  plan?: string | undefined
  onApprove: () => void
  onReject: (feedback?: string) => void
  isPending?: boolean | undefined
  error?: Error | null | undefined
}

export function PlanApprovalInput({
  plan,
  onApprove,
  onReject,
  isPending = false,
  error,
}: PlanApprovalInputProps) {
  const [showFeedback, setShowFeedback] = useState(false)
  const [feedback, setFeedback] = useState('')

  const handleRejectClick = () => {
    if (showFeedback) {
      // Already showing feedback, submit the rejection
      onReject(feedback || undefined)
    } else {
      // First click, show feedback textarea
      setShowFeedback(true)
    }
  }

  const handleCancelFeedback = () => {
    setShowFeedback(false)
    setFeedback('')
  }

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

          {/* Feedback textarea */}
          {showFeedback && (
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-sm text-muted-foreground">
                  Provide feedback for rejection
                </span>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-6 w-6"
                  aria-label="Cancel feedback"
                  onClick={handleCancelFeedback}
                  disabled={isPending}
                >
                  <X className="h-4 w-4" />
                </Button>
              </div>
              <Textarea
                placeholder="Explain what changes you'd like to see..."
                value={feedback}
                onChange={(e) => setFeedback(e.target.value)}
                className="min-h-[80px] resize-none"
                rows={3}
                disabled={isPending}
              />
            </div>
          )}

          {/* Error message */}
          {error && (
            <p className="text-sm text-destructive">{error.message || 'Failed to submit'}</p>
          )}

          {/* Action buttons */}
          <div className="flex gap-2">
            <Button
              onClick={onApprove}
              disabled={isPending || showFeedback}
              className="flex-1 gap-2"
              size="sm"
            >
              {isPending && !showFeedback ? (
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
              {isPending && showFeedback ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <XCircle className="h-4 w-4" />
              )}
              {showFeedback ? 'Submit Rejection' : 'Reject'}
            </Button>
          </div>
        </div>
      </div>
    </div>
  )
}
