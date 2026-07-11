import { Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { FeedbackSurface } from '@/features/feedback/api/feedback.api'
import { useSubmitFeedback } from '@/features/feedback/hooks/use-submit-feedback'
import { Button } from '@/lib/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/lib/components/ui/dialog'
import { Label } from '@/lib/components/ui/label'
import { Textarea } from '@/lib/components/ui/textarea'

export interface FeedbackDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  taskId: string
  messageUuid?: string | undefined
  toolUseId?: string | undefined
  surface: FeedbackSurface
  metadata?: Record<string, unknown> | undefined
}

const SURFACE_LABELS: Record<FeedbackSurface, string> = {
  task: 'task',
  message: 'assistant message',
  tool_call: 'tool call',
  preview_failure: 'preview failure',
}

export function FeedbackDialog({
  open,
  onOpenChange,
  taskId,
  messageUuid,
  toolUseId,
  surface,
  metadata,
}: FeedbackDialogProps) {
  const [comment, setComment] = useState('')
  const [error, setError] = useState<string | null>(null)
  // FeedbackDialog is instantiated per message row and per tool call, so an
  // eagerly mounted Radix Dialog tree multiplies across long transcripts and
  // re-renders on every keystroke elsewhere on the page. Mount nothing until
  // first opened; stay mounted afterwards so the close animation plays.
  const [hasOpened, setHasOpened] = useState(open)
  if (open && !hasOpened) {
    setHasOpened(true)
  }
  const submit = useSubmitFeedback()

  // Reset form fields each time the dialog re-opens. `submit.reset` is stable
  // across renders.
  useEffect(() => {
    if (open) {
      setComment('')
      setError(null)
      submit.reset()
    }
  }, [open, submit.reset])

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    const trimmed = comment.trim()
    if (!trimmed) {
      setError('Please enter a comment.')
      return
    }
    setError(null)
    try {
      await submit.mutateAsync({
        taskId,
        messageUuid,
        toolUseId,
        surface,
        comment: trimmed,
        metadata,
      })
      onOpenChange(false)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to submit feedback.')
    }
  }

  if (!hasOpened) {
    return null
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>Leave feedback</DialogTitle>
            <DialogDescription>
              Add a note about this {SURFACE_LABELS[surface]}. Used to improve agent skills,
              prompts, and tooling.
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-2 py-4">
            <Label htmlFor="feedback-comment">Comment</Label>
            <Textarea
              id="feedback-comment"
              value={comment}
              onChange={(e) => {
                setComment(e.target.value)
                if (error) {
                  setError(null)
                }
              }}
              placeholder="What went wrong, or what could be better?"
              rows={5}
              disabled={submit.isPending}
              autoFocus
            />
            {error && <p className="text-sm text-destructive">{error}</p>}
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={submit.isPending}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={submit.isPending}>
              {submit.isPending ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Submitting…
                </>
              ) : (
                'Submit'
              )}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
