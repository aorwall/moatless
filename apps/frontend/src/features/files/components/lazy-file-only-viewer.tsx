import { WorkerPoolContextProvider } from '@pierre/diffs/react'
import DiffsWorker from '@pierre/diffs/worker/worker-portable.js?worker'
import { lazy, Suspense } from 'react'

const FileOnlyViewer = lazy(() =>
  import('./file-only-viewer').then((module) => ({ default: module.FileOnlyViewer })),
)

interface LazyFileOnlyViewerProps {
  taskId: string
}

export function LazyFileOnlyViewer({ taskId }: LazyFileOnlyViewerProps) {
  return (
    <Suspense
      fallback={
        <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
          Loading file...
        </div>
      }
    >
      <WorkerPoolContextProvider
        poolOptions={{ workerFactory: () => new DiffsWorker() }}
        highlighterOptions={{}}
      >
        <FileOnlyViewer taskId={taskId} />
      </WorkerPoolContextProvider>
    </Suspense>
  )
}
