import { useCallback, useState, useRef, useEffect } from 'react'
import { FileTree } from './components/file-tree'
import { FileViewer } from './components/file-viewer'
import { SearchIcon } from './components/icons'
import { useFileSearch } from './hooks/use-file-search'

const MIN_SIDEBAR_WIDTH = 180
const MAX_SIDEBAR_WIDTH = 500
const DEFAULT_SIDEBAR_WIDTH = 280

export function App() {
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH)
  const [isResizing, setIsResizing] = useState(false)
  const [showSearch, setShowSearch] = useState(false)
  const [searchQuery, setSearchQuery] = useState('')
  const searchInputRef = useRef<HTMLInputElement>(null)
  const fileSearch = useFileSearch()

  const handleFileSelect = useCallback((filePath: string) => {
    setSelectedFile(filePath)
    setShowSearch(false)
  }, [])

  // Keyboard shortcut for search
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

  // Search on query change
  useEffect(() => {
    if (searchQuery.length >= 2) {
      fileSearch.mutate({ query: searchQuery, maxResults: 20 })
    }
  }, [searchQuery])

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    setIsResizing(true)

    const startX = e.clientX
    const startWidth = sidebarWidth

    const handleMouseMove = (e: MouseEvent) => {
      const newWidth = Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, startWidth + (e.clientX - startX)))
      setSidebarWidth(newWidth)
    }

    const handleMouseUp = () => {
      setIsResizing(false)
      document.removeEventListener('mousemove', handleMouseMove)
      document.removeEventListener('mouseup', handleMouseUp)
    }

    document.addEventListener('mousemove', handleMouseMove)
    document.addEventListener('mouseup', handleMouseUp)
  }, [sidebarWidth])

  // Listen for messages from parent iframe
  useEffect(() => {
    const handleMessage = (e: MessageEvent) => {
      if (e.data?.type === 'file-explorer:open' && e.data.path) {
        setSelectedFile(e.data.path)
      }
    }
    window.addEventListener('message', handleMessage)
    return () => window.removeEventListener('message', handleMessage)
  }, [])

  return (
    <div className="h-full flex flex-col">
      {/* Search overlay */}
      {showSearch && (
        <div className="absolute inset-0 z-50 flex justify-center pt-16" style={{ background: 'rgba(0,0,0,0.5)' }}>
          <div className="w-full max-w-md h-fit rounded-lg shadow-xl" style={{ background: 'var(--card)', border: '1px solid var(--border)' }}>
            <div className="flex items-center gap-2 p-3" style={{ borderBottom: '1px solid var(--border)' }}>
              <SearchIcon className="h-4 w-4 shrink-0" style={{ color: 'var(--muted-foreground)' }} />
              <input
                ref={searchInputRef}
                type="text"
                placeholder="Search files..."
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="flex-1 bg-transparent text-sm outline-none"
                autoFocus
              />
              <kbd className="text-xs px-1.5 py-0.5 rounded" style={{ background: 'var(--muted)', color: 'var(--muted-foreground)' }}>
                Esc
              </kbd>
            </div>
            <div className="max-h-64 overflow-auto">
              {fileSearch.data?.files.map((file) => (
                <button
                  key={file.path}
                  type="button"
                  className="w-full px-3 py-2 text-sm text-left hover:bg-[var(--accent)] transition-colors font-mono truncate"
                  onClick={() => handleFileSelect(file.path)}
                >
                  {file.path}
                </button>
              ))}
              {fileSearch.data?.files.length === 0 && searchQuery.length >= 2 && (
                <div className="p-3 text-sm text-center" style={{ color: 'var(--muted-foreground)' }}>
                  No files found
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Main layout */}
      <div className="flex-1 flex overflow-hidden" style={{ cursor: isResizing ? 'col-resize' : undefined }}>
        {/* Sidebar */}
        <div className="flex-shrink-0 overflow-hidden" style={{ width: `${sidebarWidth}px`, borderRight: '1px solid var(--border)' }}>
          <FileTree selectedPath={selectedFile} onFileSelect={handleFileSelect} />
        </div>

        {/* Resize handle */}
        <div
          className="w-1 cursor-col-resize hover:bg-[var(--ring)] transition-colors flex-shrink-0"
          onMouseDown={handleMouseDown}
          style={isResizing ? { background: 'var(--ring)' } : undefined}
        />

        {/* File viewer */}
        <div className="flex-1 min-w-0 overflow-hidden">
          <FileViewer filePath={selectedFile} />
        </div>
      </div>
    </div>
  )
}
