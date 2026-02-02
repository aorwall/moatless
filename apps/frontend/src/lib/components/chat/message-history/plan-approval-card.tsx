import type { AllowedPrompt } from '@moatless/types'
import { CheckCircle, ChevronDown, ChevronRight, FileText, Loader2, XCircle } from 'lucide-react'
import { useCallback, useState } from 'react'
import { useTaskId } from '@/features/tasks'
import { useSendMessage } from '@/features/tasks/hooks/use-tasks'
import { Response } from '@/lib/components/ai-elements/response'
import { Button } from '@/lib/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/lib/components/ui/card'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible'
import { Textarea } from '@/lib/components/ui/textarea'
import { cn } from '@/lib/utils'

interface Props {
  plan?: string
  allowedPrompts?: AllowedPrompt[]
  isResolved?: boolean
  wasApproved?: boolean
}

export function PlanApprovalCard({ plan, allowedPrompts, isResolved = false, wasApproved }: Props) {
  const taskId = useTaskId()
  const [resolved, setResolved] = useState(isResolved)
  const [approved, setApproved] = useState(wasApproved)
  const [feedback, setFeedback] = useState('')
  const [showFeedback, setShowFeedback] = useState(false)
  const [isExpanded, setIsExpanded] = useState(!isResolved)
  const submitMutation = useSendMessage(taskId)

  const handleResponse = useCallback(
    async (approve: boolean) => {
      try {
        await submitMutation.mutateAsync({
          message: '',
          toolName: 'ExitPlanMode',
          toolResponse: {
            approved: approve,
            feedback: !approve && feedback ? feedback : undefined,
          },
        })
        setResolved(true)
        setApproved(approve)
      } catch {
        // Error handled by mutation
      }
    },
    [feedback, submitMutation],
  )

  return (
    <Card
      className={cn(
        'my-2',
        resolved && approved && 'border-green-500/30 bg-green-500/5',
        resolved && !approved && 'border-red-500/30 bg-red-500/5',
        !resolved && 'border-purple-500/30 bg-purple-500/5',
      )}
    >
      <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
        <CardHeader className="pb-2">
          <CollapsibleTrigger asChild>
            <CardTitle className="text-sm font-medium flex items-center gap-2 cursor-pointer hover:opacity-80">
              {isExpanded ? (
                <ChevronDown className="h-4 w-4" />
              ) : (
                <ChevronRight className="h-4 w-4" />
              )}
              <FileText className="h-4 w-4 text-purple-500" />
              {resolved ? (
                approved ? (
                  <>
                    <CheckCircle className="h-4 w-4 text-green-500" /> Plan Approved
                  </>
                ) : (
                  <>
                    <XCircle className="h-4 w-4 text-red-500" /> Plan Rejected
                  </>
                )
              ) : (
                'Review Plan'
              )}
            </CardTitle>
          </CollapsibleTrigger>
        </CardHeader>

        <CollapsibleContent>
          <CardContent className="space-y-4">
            {plan && (
              <div className="max-h-96 overflow-y-auto rounded-md bg-muted/50 p-3">
                <Response>{plan}</Response>
              </div>
            )}

            {allowedPrompts && allowedPrompts.length > 0 && (
              <div className="space-y-1">
                <p className="text-xs font-medium text-muted-foreground">Requested Permissions:</p>
                <ul className="text-xs space-y-1">
                  {allowedPrompts.map((p) => (
                    <li key={`${p.tool}-${p.prompt}`} className="flex items-center gap-1">
                      <span className="text-muted-foreground">*</span>
                      <code className="bg-muted px-1 rounded">{p.tool}</code>
                      <span>{p.prompt}</span>
                    </li>
                  ))}
                </ul>
              </div>
            )}

            {submitMutation.error && (
              <p className="text-sm text-destructive">
                {submitMutation.error instanceof Error
                  ? submitMutation.error.message
                  : 'Failed to submit'}
              </p>
            )}

            {!resolved && (
              <div className="space-y-3">
                {showFeedback && (
                  <Textarea
                    placeholder="Optional feedback for rejection..."
                    value={feedback}
                    onChange={(e) => setFeedback(e.target.value)}
                    className="text-sm"
                    rows={2}
                  />
                )}

                <div className="flex gap-2">
                  <Button
                    onClick={() => handleResponse(true)}
                    disabled={submitMutation.isPending}
                    className="flex-1"
                    size="sm"
                  >
                    {submitMutation.isPending ? (
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    ) : (
                      <CheckCircle className="mr-2 h-4 w-4" />
                    )}
                    Approve
                  </Button>
                  <Button
                    onClick={() => (showFeedback ? handleResponse(false) : setShowFeedback(true))}
                    disabled={submitMutation.isPending}
                    variant="outline"
                    className="flex-1"
                    size="sm"
                  >
                    {submitMutation.isPending ? (
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                    ) : (
                      <XCircle className="mr-2 h-4 w-4" />
                    )}
                    {showFeedback ? 'Reject with Feedback' : 'Reject'}
                  </Button>
                </div>
              </div>
            )}
          </CardContent>
        </CollapsibleContent>
      </Collapsible>
    </Card>
  )
}
