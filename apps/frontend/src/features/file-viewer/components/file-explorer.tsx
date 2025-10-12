import { useState } from 'react'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { FileExplorerHeader } from './file-explorer-header'
import { FileTree } from './file-tree'
import { FileViewer } from './file-viewer'

interface FileExplorerProps {
  taskId: string
}

export function FileExplorer({ taskId }: FileExplorerProps) {
  const [selectedFile, setSelectedFile] = useState<string>('')

  return (
    <div className="flex flex-col h-full">
      <FileExplorerHeader />
      <div className="flex-1 overflow-hidden">
        <ResizablePanelGroup direction="horizontal" className="h-full">
          <ResizablePanel defaultSize={25} minSize={15} maxSize={50}>
            <FileTree taskId={taskId} onFileSelect={setSelectedFile} />
          </ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize={75}>
            <FileViewer taskId={taskId} filePath={selectedFile} />
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>
    </div>
  )
}
