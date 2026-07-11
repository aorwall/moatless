import { lazy, Suspense } from 'react'

const FileExplorerWithWorkerPool = lazy(() => import('./file-explorer-with-worker-pool'))

interface LazyFileExplorerProps {
  taskId: string
}

/**
 * Code-split entry point for the file explorer. @pierre/diffs (worker +
 * highlighter) is several MB, so it is only fetched when a task's files view
 * is actually opened.
 */
export function LazyFileExplorer({ taskId }: LazyFileExplorerProps) {
  return (
    <Suspense
      fallback={
        <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
          Loading file explorer...
        </div>
      }
    >
      <FileExplorerWithWorkerPool taskId={taskId} />
    </Suspense>
  )
}
