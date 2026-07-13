import { MessageSquarePlus, XCircle } from 'lucide-react'
import { useState } from 'react'
import { FeedbackDialog } from '@/features/feedback/components/feedback-dialog'
import { Button } from '@/lib/components/ui/button'

/** Overlay shown when the preview proxy cannot reach the dev server. */
export function ProxyErrorOverlay({
  taskId,
  onReload,
  serverName,
  serverLabel,
  previewUrl,
}: {
  taskId?: string | undefined
  onReload: () => void
  serverName?: string | undefined
  serverLabel?: string | undefined
  previewUrl?: string | undefined
}) {
  const [feedbackOpen, setFeedbackOpen] = useState(false)
  const errorMessage =
    'Could not connect to the preview server. It may have crashed or be listening on a different port.'
  return (
    <div className="absolute inset-0 flex items-center justify-center z-10 bg-background/80 backdrop-blur-sm">
      <div className="flex flex-col items-center justify-center text-center space-y-4 p-8">
        <XCircle className="h-12 w-12 text-destructive" />
        <h3 className="text-lg font-semibold">Connection Failed</h3>
        <p className="text-sm text-muted-foreground max-w-md">{errorMessage}</p>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={onReload}>
            Retry
          </Button>
          {taskId && (
            <Button variant="outline" size="sm" onClick={() => setFeedbackOpen(true)}>
              <MessageSquarePlus className="h-4 w-4 mr-2" />
              Give feedback
            </Button>
          )}
        </div>
      </div>
      {taskId && (
        <FeedbackDialog
          open={feedbackOpen}
          onOpenChange={setFeedbackOpen}
          taskId={taskId}
          surface="preview_failure"
          metadata={{
            preview_error: errorMessage,
            server_name: serverName,
            server_label: serverLabel,
            preview_url: previewUrl,
          }}
        />
      )}
    </div>
  )
}
