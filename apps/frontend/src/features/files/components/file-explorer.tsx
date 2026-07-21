import { PanelLeftOpen, Search } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { useIsMobile } from '@/lib/hooks/use-mobile'
import { useFileSearch } from '../hooks/use-file-search'
import { useFileViewState } from '../hooks/use-file-view-state'
import { useSelectedFile } from '../hooks/use-selected-file'
import '../styles/file-explorer.css'
import { FileTree } from './file-tree'
import { FileViewer } from './file-viewer'

const MIN_SIDEBAR_WIDTH = 180
const MAX_SIDEBAR_WIDTH = 500
const DEFAULT_SIDEBAR_WIDTH = 280

interface FileExplorerProps {
  taskId: string
}

export function FileExplorer({ taskId }: FileExplorerProps) {
  const { selectedFile, setSelectedFile } = useSelectedFile()
  const { fileView, setFileView, isExplorerCollapsed, setExplorerCollapsed } = useFileViewState()
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH)
  const [isResizing, setIsResizing] = useState(false)
  const [showSearch, setShowSearch] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const searchInputRef = useRef<HTMLInputElement>(null)
  const fileSearch = useFileSearch(taskId)
  const isMobile = useIsMobile()

  const handleFileSelect = (filePath: string) => {
    setSelectedFile(filePath)
    setShowSearch(false)
  }

  // On mobile the tree and viewer are a single swappable pane — once a file is
  // selected, collapse the tree so the viewer takes over the screen. Driven by
  // the selected-file state (not the tree's selection callback, which captures a
  // stale `isMobile`) and after the file param commits, so the two URL updates
  // sequence instead of racing.
  useEffect(() => {
    if (isMobile && selectedFile !== undefined) {
      setExplorerCollapsed(true)
    }
  }, [isMobile, selectedFile, setExplorerCollapsed])

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'p') {
        e.preventDefault()
        setShowSearch((prev) => !prev)
        setTimeout(() => searchInputRef.current?.focus(), 50)
      }
      if (e.key === 'Escape') {
        setShowSearch(false)
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [])

  useEffect(() => {
    if (searchQuery.length >= 2) {
      fileSearch.mutate({ query: searchQuery, maxResults: 20 })
    }
  }, [searchQuery, fileSearch.mutate])

  const handleMouseDown = (e: React.MouseEvent) => {
    e.preventDefault()
    setIsResizing(true)

    const startX = e.clientX
    const startWidth = sidebarWidth

    const handleMouseMove = (moveEvent: MouseEvent) => {
      const newWidth = Math.max(
        MIN_SIDEBAR_WIDTH,
        Math.min(MAX_SIDEBAR_WIDTH, startWidth + (moveEvent.clientX - startX)),
      )
      setSidebarWidth(newWidth)
    }

    const handleMouseUp = () => {
      setIsResizing(false)
      document.removeEventListener('mousemove', handleMouseMove)
      document.removeEventListener('mouseup', handleMouseUp)
    }

    document.addEventListener('mousemove', handleMouseMove)
    document.addEventListener('mouseup', handleMouseUp)
  }

  const handleResizeKeyDown = (e: React.KeyboardEvent<HTMLButtonElement>) => {
    const delta = e.key === 'ArrowLeft' ? -10 : e.key === 'ArrowRight' ? 10 : 0
    if (delta === 0) {
      return
    }

    e.preventDefault()
    setSidebarWidth((width) =>
      Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, width + delta)),
    )
  }

  return (
    <div className="h-full flex flex-col relative">
      {showSearch && (
        <div className="absolute inset-0 z-50 flex justify-center pt-16 bg-black/50">
          <div className="w-full max-w-md h-fit rounded-lg shadow-xl bg-card border">
            <div className="flex items-center gap-2 p-3 border-b">
              <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
              <input
                ref={searchInputRef}
                type="text"
                placeholder="Search files..."
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="flex-1 bg-transparent text-sm outline-none"
              />
              <kbd className="text-xs px-1.5 py-0.5 rounded bg-muted text-muted-foreground">
                Esc
              </kbd>
            </div>
            <div className="max-h-64 overflow-auto">
              {fileSearch.data?.files.map((file) => (
                <button
                  key={file.path}
                  type="button"
                  className="w-full px-3 py-2 text-sm text-left hover:bg-accent transition-colors font-mono truncate"
                  onClick={() => handleFileSelect(file.path)}
                >
                  {file.path}
                </button>
              ))}
              {fileSearch.data?.files.length === 0 && searchQuery.length >= 2 && (
                <div className="p-3 text-sm text-center text-muted-foreground">No files found</div>
              )}
            </div>
          </div>
        </div>
      )}

      <div
        className="flex-1 flex overflow-hidden"
        style={{ cursor: isResizing ? 'col-resize' : undefined }}
      >
        {!isExplorerCollapsed && (
          <>
            <div
              className={
                isMobile
                  ? 'flex-1 min-w-0 overflow-hidden'
                  : 'flex-shrink-0 overflow-hidden border-r'
              }
              style={isMobile ? undefined : { width: `${sidebarWidth}px` }}
            >
              <FileTree
                taskId={taskId}
                selectedPath={selectedFile}
                onFileSelect={handleFileSelect}
              />
            </div>

            {/* Desktop-only resize handle; on mobile the tree is full-width. */}
            {!isMobile && (
              <button
                type="button"
                aria-label={`Resize file tree, currently ${sidebarWidth} pixels wide`}
                className="w-1 cursor-col-resize border-0 p-0 hover:bg-ring transition-colors flex-shrink-0"
                onMouseDown={handleMouseDown}
                onKeyDown={handleResizeKeyDown}
                style={isResizing ? { background: 'var(--ring)' } : undefined}
              />
            )}
          </>
        )}

        {/* On mobile the viewer only shows when the tree is collapsed (single
            pane); on desktop it always sits beside the tree. */}
        {(!isMobile || isExplorerCollapsed) && (
          <div className="flex-1 min-w-0 overflow-hidden">
            {/* When the tree is collapsed, the expand toggle rides on the file
                viewer's header row (same row as the file name). */}
            <FileViewer
              taskId={taskId}
              filePath={selectedFile}
              viewMode={fileView}
              onViewModeChange={setFileView}
              hideViewModeControls={!isMobile}
              leadingControl={
                isExplorerCollapsed ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="h-8 w-8 flex-shrink-0"
                    onClick={() => setExplorerCollapsed(false)}
                    aria-label="Expand file tree"
                  >
                    <PanelLeftOpen className="h-4 w-4" />
                  </Button>
                ) : undefined
              }
            />
          </div>
        )}
      </div>
    </div>
  )
}
