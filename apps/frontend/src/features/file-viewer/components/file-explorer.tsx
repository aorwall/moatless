import { ChevronRight } from 'lucide-react'
import { useRef, useState } from 'react'
import type { ImperativePanelHandle } from 'react-resizable-panels'
import { Button } from '@/lib/components/ui/button'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { cn } from '@/lib/utils'
import { FileExplorerHeader } from './file-explorer-header'
import { FileTree } from './file-tree'
import { FileViewer } from './file-viewer'

interface FileExplorerProps {
  taskId: string
  serverManagement?: boolean
}

export function FileExplorer({ taskId, serverManagement }: FileExplorerProps) {
  const [selectedFile, setSelectedFile] = useState<string>('')
  const fileTreePanelRef = useRef<ImperativePanelHandle>(null)
  const [isFileTreeCollapsed, setIsFileTreeCollapsed] = useState(false)

  const handleExpandFileTree = () => {
    fileTreePanelRef.current?.expand()
  }

  return (
    <div className="flex flex-col h-full">
      <FileExplorerHeader serverManagement={serverManagement} />
      <div className="flex-1 overflow-hidden relative">
        <ResizablePanelGroup direction="horizontal" className="h-full">
          <ResizablePanel
            ref={fileTreePanelRef}
            defaultSize={25}
            minSize={5}
            maxSize={50}
            collapsible={true}
            collapsedSize={0}
            onCollapse={() => setIsFileTreeCollapsed(true)}
            onExpand={() => setIsFileTreeCollapsed(false)}
          >
            <FileTree taskId={taskId} onFileSelect={setSelectedFile} />
          </ResizablePanel>
          <ResizableHandle
            withHandle
            className={cn(
              isFileTreeCollapsed && 'bg-primary/20 w-1 hover:bg-primary/40 transition-colors',
            )}
          />
          <ResizablePanel defaultSize={75}>
            <FileViewer taskId={taskId} filePath={selectedFile} />
          </ResizablePanel>
        </ResizablePanelGroup>

        {/* Expand button for collapsed file tree panel */}
        {isFileTreeCollapsed && (
          <Button
            onClick={handleExpandFileTree}
            className="absolute left-2 top-2 z-10"
            size="sm"
            variant="outline"
            aria-label="Expand file tree panel"
          >
            <ChevronRight className="h-4 w-4 mr-1" />
            Files
          </Button>
        )}
      </div>
    </div>
  )
}
