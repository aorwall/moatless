import '@moatless/diff-viewer/styles/diff-theme.css'

import { type DiffFile, parseDiff } from '@moatless/diff-viewer'
import { ChevronDown, ChevronRight, GitBranch, Loader2 } from 'lucide-react'
import { useMemo } from 'react'
import { useTaskGitDiffQuery } from '@/features/tasks/hooks/use-task-queries'
import { Button } from '@/lib/components/ui/button'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { useDiffState } from '../hooks/use-diff-state'
import { DiffContent } from './diff-content'
import { DiffSidebar } from './diff-sidebar'

interface DiffViewerProps {
  taskId: string
}

/**
 * Main diff viewer component for displaying git changes
 */
export function DiffViewer({ taskId }: DiffViewerProps) {
  // Fetch git diff data
  const { data, isLoading, error } = useTaskGitDiffQuery(taskId)

  // Parse the diff string into structured format
  const parsedFiles: DiffFile[] = useMemo(() => {
    if (!data?.diff) {
      return []
    }
    const parsed = parseDiff(data.diff)
    return parsed.files
  }, [data?.diff])

  // State management
  const filenames = useMemo(() => parsedFiles.map((f) => f.filename), [parsedFiles])
  const {
    state,
    toggleFileCollapsed,
    setSelectedFile,
    setSearchQuery,
    isFileCollapsed,
    collapseAll,
    expandAll,
  } = useDiffState(filenames)

  // Handle file selection from sidebar
  const handleFileSelect = (filename: string) => {
    setSelectedFile(filename)
    // Expand the file if it's collapsed
    if (isFileCollapsed(filename)) {
      toggleFileCollapsed(filename)
    }
  }

  // Loading state
  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="flex flex-col items-center gap-3">
          <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
          <span className="text-sm text-muted-foreground">Loading diff...</span>
        </div>
      </div>
    )
  }

  // Error state
  if (error) {
    return (
      <div className="p-4">
        <ErrorAlert error={error} />
      </div>
    )
  }

  // No changes state
  if (parsedFiles.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-4 text-muted-foreground">
        <GitBranch className="h-12 w-12" />
        <div className="text-center">
          <p className="text-lg font-medium">No changes</p>
          <p className="text-sm">There are no uncommitted changes to display</p>
        </div>
      </div>
    )
  }

  return (
    <div className="flex h-full">
      {/* Sidebar */}
      <div className="w-64 shrink-0">
        <DiffSidebar
          files={parsedFiles}
          selectedFile={state.selectedFile}
          onFileSelect={handleFileSelect}
          searchQuery={state.searchQuery}
          onSearchChange={setSearchQuery}
        />
      </div>

      {/* Main content */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Toolbar */}
        <div className="flex items-center gap-2 px-4 py-2 border-b border-border bg-muted/20">
          <Button variant="ghost" size="sm" onClick={expandAll} className="h-7 text-xs">
            <ChevronDown className="h-3.5 w-3.5 mr-1" />
            Expand all
          </Button>
          <Button variant="ghost" size="sm" onClick={collapseAll} className="h-7 text-xs">
            <ChevronRight className="h-3.5 w-3.5 mr-1" />
            Collapse all
          </Button>
        </div>

        {/* Diff content */}
        <div className="flex-1 min-h-0">
          <DiffContent
            files={parsedFiles}
            selectedFile={state.selectedFile}
            isFileCollapsed={isFileCollapsed}
            onToggleCollapsed={toggleFileCollapsed}
          />
        </div>
      </div>
    </div>
  )
}
