import { Link } from '@tanstack/react-router'
import { SandboxStatusOverlay } from '@/features/preview'
import { useSandboxRetry, useTaskSandboxStatusQuery } from '@/lib/api/task-sandbox-queries'
import { useFileContent } from '../hooks/use-file-content'
import { useSelectedFile } from '../hooks/use-selected-file'
import { isHtmlFile, isMarkdownFile } from './file-viewer-utils'
import { HtmlPreview } from './html-preview'
import { MarkdownPreview } from './markdown-preview'

interface StandalonePreviewProps {
  taskId: string
}

function CenteredMessage({ title, detail }: { title: string; detail?: string }) {
  return (
    <div className="h-full flex items-center justify-center p-8">
      <div className="text-center space-y-2">
        <p className="text-sm font-medium">{title}</p>
        {detail && <p className="text-xs text-muted-foreground">{detail}</p>}
      </div>
    </div>
  )
}

/**
 * Chrome-free rendering of a single previewable file (`?file=`), for sharing a
 * report as a page of its own. HTML still renders in a sandboxed frame — the
 * file comes from the workspace and would otherwise run its scripts with the
 * app's origin and session.
 */
export function StandalonePreview({ taskId }: StandalonePreviewProps) {
  const { selectedFile } = useSelectedFile()
  const sandboxStatusQuery = useTaskSandboxStatusQuery(taskId)
  const retryMutation = useSandboxRetry(taskId)

  const sandboxStatus = sandboxStatusQuery.data?.sandboxStatus
  const displayStatus = sandboxStatus ?? (sandboxStatusQuery.error ? 'error' : undefined)
  const sandboxError = sandboxStatusQuery.data?.sandboxError ?? sandboxStatusQuery.error?.message
  const isSandboxReady = displayStatus === 'ready'

  const { data, isLoading, error } = useFileContent(taskId, selectedFile, false)

  if (!isSandboxReady) {
    return (
      <div className="h-full w-full relative bg-background">
        {displayStatus ? (
          <SandboxStatusOverlay
            sandboxStatus={displayStatus}
            sandboxError={sandboxError}
            onRetry={() => retryMutation.mutate()}
            isRetrying={retryMutation.isPending}
            retryError={retryMutation.error}
          />
        ) : (
          <CenteredMessage title="Checking sandbox status..." />
        )}
      </div>
    )
  }

  if (!selectedFile) {
    return (
      <CenteredMessage
        title="No file selected"
        detail="Add ?file=<path> to the URL to preview a file."
      />
    )
  }

  if (isLoading) {
    return <CenteredMessage title="Loading preview..." detail={selectedFile} />
  }

  if (error) {
    return <CenteredMessage title="Failed to load file" detail={error.message} />
  }

  if (!data || data.isBinary) {
    return <CenteredMessage title="No preview available" detail={selectedFile} />
  }

  if (isHtmlFile(selectedFile)) {
    return <HtmlPreview content={data.content} filePath={selectedFile} />
  }

  if (isMarkdownFile(selectedFile)) {
    return (
      <div className="h-full w-full overflow-auto bg-background">
        <MarkdownPreview content={data.content} taskId={taskId} filePath={selectedFile} />
      </div>
    )
  }

  return (
    <div className="h-full flex items-center justify-center p-8">
      <div className="text-center space-y-2">
        <p className="text-sm font-medium">No preview for this file type</p>
        <Link
          to="/tasks/$task-id/file"
          params={{ 'task-id': taskId }}
          search={{ file: selectedFile }}
          className="text-xs text-primary hover:underline"
        >
          Open {selectedFile} in the file viewer
        </Link>
      </div>
    </div>
  )
}
