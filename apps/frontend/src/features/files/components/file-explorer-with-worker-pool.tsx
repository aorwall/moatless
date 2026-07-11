import { WorkerPoolContextProvider } from '@pierre/diffs/react'
import DiffsWorker from '@pierre/diffs/worker/worker-portable.js?worker'

import { FileExplorer } from './file-explorer'

interface FileExplorerWithWorkerPoolProps {
  taskId: string
}

/**
 * Bundles the diff worker pool together with the file explorer so the whole
 * @pierre/diffs dependency stays in this lazily-loaded chunk (see
 * `LazyFileExplorer`) instead of the app entry bundle.
 */
export default function FileExplorerWithWorkerPool({ taskId }: FileExplorerWithWorkerPoolProps) {
  return (
    <WorkerPoolContextProvider
      poolOptions={{ workerFactory: () => new DiffsWorker() }}
      highlighterOptions={{}}
    >
      <FileExplorer taskId={taskId} />
    </WorkerPoolContextProvider>
  )
}
