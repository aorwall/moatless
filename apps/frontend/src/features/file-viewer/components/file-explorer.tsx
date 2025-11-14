import { ChevronRight, FolderOpen } from 'lucide-react'
import { useRef, useState } from 'react'
import type { ImperativePanelHandle } from 'react-resizable-panels'
import { useSelectedFile } from '@/features/tasks'
import { Button } from '@/lib/components/ui/button'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { Sheet, SheetContent, SheetHeader, SheetTitle } from '@/lib/components/ui/sheet'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { cn } from '@/lib/utils'
import { FileExplorerHeader } from './file-explorer-header'
import { FileTree } from './file-tree'
import { FileViewer } from './file-viewer'

interface FileExplorerProps {
  taskId: string
  serverManagement?: boolean
}

export function FileExplorer({ taskId, serverManagement }: FileExplorerProps) {
  const { selectedFile, setSelectedFile } = useSelectedFile()
  const fileTreePanelRef = useRef<ImperativePanelHandle>(null)
  const [isFileTreeCollapsed, setIsFileTreeCollapsed] = useState(false)
  const isMobile = useIsMobile()
  const [mobileTreeOpen, setMobileTreeOpen] = useState(false)

  const handleExpandFileTree = () => {
    fileTreePanelRef.current?.expand()
  }

  const handleFileSelect = (filePath: string) => {
    setSelectedFile(filePath)
    // Auto-close tree on mobile when file is selected
    if (isMobile) {
      setMobileTreeOpen(false)
    }
  }

  // Mobile layout: Sheet drawer for file tree, full-screen file viewer
  if (isMobile) {
    return (
      <div className="flex flex-col h-full relative">
        {/* File viewer takes full screen */}
        <FileViewer taskId={taskId} filePath={selectedFile || ''} />

        {/* Floating button to open file tree */}
        <Button
          onClick={() => setMobileTreeOpen(true)}
          className="fixed left-4 bottom-4 z-10 shadow-lg"
          size="default"
          aria-label="Browse files"
        >
          <FolderOpen className="h-4 w-4 mr-2" />
          Browse Files
        </Button>

        {/* File tree as slide-out Sheet */}
        <Sheet open={mobileTreeOpen} onOpenChange={setMobileTreeOpen}>
          <SheetContent side="left" className="w-[85vw] sm:w-[400px] p-0">
            <SheetHeader className="p-4 border-b">
              <SheetTitle>Files</SheetTitle>
            </SheetHeader>
            <div className="h-[calc(100vh-80px)] overflow-auto">
              <FileTree taskId={taskId} onFileSelect={handleFileSelect} />
            </div>
          </SheetContent>
        </Sheet>
      </div>
    )
  }

  // Desktop layout: existing ResizablePanel structure
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
            <FileViewer taskId={taskId} filePath={selectedFile || ''} />
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
