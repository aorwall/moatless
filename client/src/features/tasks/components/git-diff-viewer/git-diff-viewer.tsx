import { ChevronDown, ChevronRight, FileIcon, Loader2 } from 'lucide-react'
import { useMemo, useState } from 'react'
import { type ChangeData, Diff, type FileData, Hunk, parseDiff } from 'react-diff-view'
import 'react-diff-view/style/index.css'
import 'prism-color-variables/variables.css'

import { useGitDiff, useSelectedIteration } from '@/features/tasks/contexts/task-providers'
import { useTaskLoadingState } from '@/features/tasks/hooks/optimized-hooks'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { cn } from '@/lib/utils'
import './github-diff.css'
import './prism-theme.css'
import { tokenizeHunks } from './syntax-highlighter'

interface GitDiffViewerProps {
  className?: string
}

// Use the proper FileData type from react-diff-view
type ParsedFile = FileData

interface FileDiffProps {
  file: ParsedFile
  isExpanded: boolean
  onToggle: () => void
}

function FileDiff({ file, isExpanded, onToggle }: FileDiffProps) {
  const { oldRevision, newRevision, type, hunks, oldPath, newPath } = file
  const filePath = newPath || oldPath || newRevision || oldRevision

  // Calculate additions and deletions
  const stats = useMemo(() => {
    let additions = 0
    let deletions = 0

    hunks.forEach((hunk) => {
      hunk.changes.forEach((change: ChangeData) => {
        if (change.type === 'insert') {
          additions++
        }
        if (change.type === 'delete') {
          deletions++
        }
      })
    })

    return { additions, deletions }
  }, [hunks])

  // Tokenize hunks for syntax highlighting
  const tokens = useMemo(() => {
    return isExpanded ? tokenizeHunks(hunks, filePath) : undefined
  }, [hunks, filePath, isExpanded])

  return (
    <div className="github-diff-file border border-border rounded-lg mb-4 overflow-hidden">
      {/* File Header */}
      <button
        type="button"
        className="github-diff-header bg-muted/50 px-4 py-2 flex items-center justify-between cursor-pointer hover:bg-muted/70 transition-colors w-full text-left"
        onClick={onToggle}
      >
        <div className="flex items-center gap-2">
          <span className="p-0 hover:bg-muted rounded">
            {isExpanded ? (
              <ChevronDown className="h-4 w-4" />
            ) : (
              <ChevronRight className="h-4 w-4" />
            )}
          </span>
          <FileIcon className="h-4 w-4 text-muted-foreground" />
          <span className="font-mono text-sm">{filePath}</span>
        </div>
        <div className="flex items-center gap-3 text-sm">
          <span className="text-green-600 dark:text-green-400">+{stats.additions}</span>
          <span className="text-red-600 dark:text-red-400">−{stats.deletions}</span>
        </div>
      </button>

      {/* Diff Content */}
      {isExpanded && (
        <div className="github-diff-content">
          <Diff
            viewType="unified"
            diffType={type}
            hunks={hunks}
            className="diff-view"
            tokens={tokens}
            optimizeSelection
            renderToken={(token: unknown, defaultRender: (token: unknown) => React.ReactNode) => {
              // Handle prism tokens with className for syntax highlighting
              if (token.type === 'prism' && token.className) {
                return <span className={token.className}>{token.value || ''}</span>
              }
              return defaultRender(token)
            }}
          >
            {(hunks) => hunks.map((hunk) => <Hunk key={hunk.content} hunk={hunk} />)}
          </Diff>
        </div>
      )}
    </div>
  )
}

export function GitDiffViewer({ className }: GitDiffViewerProps) {
  const gitDiff = useGitDiff()
  // Use optimized hook that only subscribes to loading/error state
  const { isLoading, error } = useTaskLoadingState()
  const _selectedIteration = useSelectedIteration()
  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(new Set())

  const parsedFiles = useMemo<ParsedFile[]>(() => {
    if (!gitDiff?.diff) return []
    try {
      const files = parseDiff(gitDiff.diff) as ParsedFile[]
      // Auto-expand all files by default
      const allFileKeys = new Set(
        files.map((f) => `${f.oldPath || f.oldRevision}-${f.newPath || f.newRevision}`),
      )
      setExpandedFiles(allFileKeys)
      return files
    } catch (error) {
      console.error('Failed to parse git diff:', error)
      return []
    }
  }, [gitDiff?.diff])

  const toggleFileExpansion = (fileKey: string) => {
    setExpandedFiles((prev) => {
      const next = new Set(prev)
      if (next.has(fileKey)) {
        next.delete(fileKey)
      } else {
        next.add(fileKey)
      }
      return next
    })
  }

  // Calculate total stats from parsed files
  const totalStats = useMemo(() => {
    let totalAdditions = 0
    let totalDeletions = 0

    parsedFiles.forEach((file) => {
      file.hunks.forEach((hunk) => {
        hunk.changes.forEach((change: ChangeData) => {
          if (change.type === 'insert') {
            totalAdditions++
          }
          if (change.type === 'delete') {
            totalDeletions++
          }
        })
      })
    })

    return { totalAdditions, totalDeletions }
  }, [parsedFiles])

  if (isLoading) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
        <Loader2 className="h-8 w-8 animate-spin mb-2" data-testid="loading-spinner" />
        <p>Loading diff...</p>
      </div>
    )
  }

  if (error) {
    return (
      <div className="flex items-center justify-center h-full p-8">
        <ErrorAlert error={error} title="Failed to load git diff" className="max-w-md" />
      </div>
    )
  }

  if (!gitDiff?.diff) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
        <FileIcon className="h-12 w-12 mb-2" />
        <p>No changes to display</p>
      </div>
    )
  }

  if (parsedFiles.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
        <FileIcon className="h-12 w-12 mb-2" />
        <p>Unable to parse diff</p>
      </div>
    )
  }

  return (
    <ScrollArea className={cn('h-full overflow-auto p-4', className)}>
      {/* Summary Stats */}
      {gitDiff && parsedFiles.length > 0 && (
        <div className="mb-4 text-sm text-muted-foreground">
          Showing {parsedFiles.length} changed files with{' '}
          <span className="text-green-600 dark:text-green-400 font-medium">
            {totalStats.totalAdditions} additions
          </span>{' '}
          and{' '}
          <span className="text-red-600 dark:text-red-400 font-medium">
            {totalStats.totalDeletions} deletions
          </span>
        </div>
      )}

      {/* File Diffs */}
      {parsedFiles.map((file) => {
        const fileKey = `${file.oldPath || file.oldRevision}-${file.newPath || file.newRevision}`
        return (
          <FileDiff
            key={fileKey}
            file={file}
            isExpanded={expandedFiles.has(fileKey)}
            onToggle={() => toggleFileExpansion(fileKey)}
          />
        )
      })}
    </ScrollArea>
  )
}
