import { useFileViewState } from '../hooks/use-file-view-state'
import { useSelectedFile } from '../hooks/use-selected-file'
import { FileViewer } from './file-viewer'

interface FileOnlyViewerProps {
  taskId: string
}

export function FileOnlyViewer({ taskId }: FileOnlyViewerProps) {
  const { selectedFile } = useSelectedFile()
  const { fileView, setFileView } = useFileViewState()

  return (
    <div className="h-full w-full bg-background">
      <FileViewer
        taskId={taskId}
        filePath={selectedFile}
        viewMode={fileView}
        onViewModeChange={setFileView}
        hideHeaderActions
      />
    </div>
  )
}
