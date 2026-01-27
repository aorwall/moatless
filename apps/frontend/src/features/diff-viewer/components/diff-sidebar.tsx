import type { DiffFile } from '@moatless/diff-viewer'
import { Search } from 'lucide-react'
import { useMemo } from 'react'
import { Input } from '@/lib/components/ui/input'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { DiffSidebarItem } from './diff-sidebar-item'

interface DiffSidebarProps {
  files: DiffFile[]
  selectedFile: string | undefined
  onFileSelect: (filename: string) => void
  searchQuery: string
  onSearchChange: (query: string) => void
}

/**
 * Sidebar showing list of changed files with search
 */
export function DiffSidebar({
  files,
  selectedFile,
  onFileSelect,
  searchQuery,
  onSearchChange,
}: DiffSidebarProps) {
  // Filter files based on search query
  const filteredFiles = useMemo(() => {
    if (!searchQuery.trim()) {
      return files
    }
    const query = searchQuery.toLowerCase()
    return files.filter((file) => file.filename.toLowerCase().includes(query))
  }, [files, searchQuery])

  // Calculate totals
  const totalAdditions = files.reduce((sum, f) => sum + f.additions, 0)
  const totalDeletions = files.reduce((sum, f) => sum + f.deletions, 0)

  return (
    <div className="flex flex-col h-full border-r border-border bg-muted/20">
      {/* Header with stats */}
      <div className="px-3 py-2 border-b border-border">
        <div className="flex items-center justify-between text-sm">
          <span className="font-medium">{files.length} files changed</span>
          <span className="font-mono text-xs">
            <span className="text-green-600 dark:text-green-400">+{totalAdditions}</span>/
            <span className="text-red-600 dark:text-red-400">-{totalDeletions}</span>
          </span>
        </div>
      </div>

      {/* Search input */}
      <div className="px-3 py-2 border-b border-border">
        <div className="relative">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
          <Input
            type="text"
            placeholder="Filter files..."
            value={searchQuery}
            onChange={(e) => onSearchChange(e.target.value)}
            className="h-8 pl-8 text-sm"
          />
        </div>
      </div>

      {/* File list */}
      <ScrollArea className="flex-1">
        <div className="py-1">
          {filteredFiles.length === 0 ? (
            <div className="px-3 py-4 text-center text-sm text-muted-foreground">
              {searchQuery ? 'No matching files' : 'No files changed'}
            </div>
          ) : (
            filteredFiles.map((file) => (
              <DiffSidebarItem
                key={file.filename}
                file={file}
                isSelected={selectedFile === file.filename}
                onClick={() => onFileSelect(file.filename)}
              />
            ))
          )}
        </div>
      </ScrollArea>
    </div>
  )
}
