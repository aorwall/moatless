import { ChevronDown, ChevronRight, FileIcon, Loader2 } from 'lucide-react'
import React, { useMemo, useRef } from 'react'
import { type ChangeData, Diff, type FileData, Hunk, parseDiff } from 'react-diff-view'
import 'react-diff-view/style/index.css'
import 'prism-color-variables/variables.css'

import { useGitDiffWithLoading } from '@/features/tasks/contexts/task-providers'
import { useGitDiffState } from '@/features/tasks/contexts/task-ui-provider'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/lib/components/ui/resizable'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { cn } from '@/lib/utils'
import './github-diff.css'
import './prism-theme.css'
import { GitDiffFileTree } from './git-diff-file-tree'
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
  fileKey: string
}

function FileDiff({ file, isExpanded, onToggle, fileKey }: FileDiffProps) {
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
    <div
      className="github-diff-file border border-border rounded-lg mb-4 overflow-hidden"
      data-file-key={fileKey}
    >
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
          <span className="text-status-success">+{stats.additions}</span>
          <span className="text-status-error">−{stats.deletions}</span>
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
            renderToken={(token, defaultRender) => {
              // Handle prism tokens with className for syntax highlighting
              const prismToken = token as { type?: string; className?: string; value?: string }
              if (prismToken.type === 'prism' && prismToken.className) {
                return <span className={prismToken.className}>{prismToken.value || ''}</span>
              }
              return defaultRender(token, 0)
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
  const { gitDiff, isLoading, error } = useGitDiffWithLoading()
  const { gitDiffState, updateGitDiffState } = useGitDiffState()
  const { expandedFiles, selectedFileKey } = gitDiffState
  const scrollAreaRef = useRef<HTMLDivElement>(null)

  const parsedFiles = useMemo<ParsedFile[]>(() => {
    if (!gitDiff?.diff) return []
    try {
      const files = parseDiff(gitDiff.diff) as ParsedFile[]
      return files
    } catch (error) {
      console.error('Failed to parse git diff:', error)
      return []
    }
  }, [gitDiff?.diff])

  // Derive all file keys from parsed files
  const allFileKeys = useMemo(() => {
    return new Set(
      parsedFiles.map((f) => `${f.oldPath || f.oldRevision}-${f.newPath || f.newRevision}`),
    )
  }, [parsedFiles])

  // Track if we've auto-expanded for this set of files
  const autoExpandRef = React.useRef<Set<string>>(new Set())

  // Derive expanded files state - auto-expand if we haven't done so for these files
  const effectiveExpandedFiles = React.useMemo(() => {
    if (parsedFiles.length === 0) return expandedFiles

    // Create a stable key for this set of files
    const filesKey = Array.from(allFileKeys).sort().join('|')

    // If we haven't auto-expanded these files yet and no files are expanded
    if (!autoExpandRef.current.has(filesKey) && expandedFiles.size === 0) {
      autoExpandRef.current.add(filesKey)
      // Auto-expand all files
      return allFileKeys
    }

    return expandedFiles
  }, [parsedFiles, allFileKeys, expandedFiles])

  // Sync the derived state back to the context if it differs
  React.useEffect(() => {
    if (effectiveExpandedFiles !== expandedFiles) {
      updateGitDiffState({ expandedFiles: effectiveExpandedFiles })
    }
  }, [effectiveExpandedFiles, expandedFiles, updateGitDiffState])

  const toggleFileExpansion = (fileKey: string) => {
    const next = new Set(effectiveExpandedFiles)
    if (next.has(fileKey)) {
      next.delete(fileKey)
    } else {
      next.add(fileKey)
    }
    updateGitDiffState({ expandedFiles: next })
  }

  const handleFileSelect = (fileKey: string) => {
    updateGitDiffState({ selectedFileKey: fileKey })
    // Scroll to the selected file
    if (scrollAreaRef.current) {
      const fileElement = scrollAreaRef.current.querySelector(`[data-file-key="${fileKey}"]`)
      if (fileElement) {
        fileElement.scrollIntoView({ behavior: 'smooth', block: 'start' })
      }
    }
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
    <ResizablePanelGroup direction="horizontal" className={cn('h-full', className)}>
      {/* Left Panel - File Tree */}
      <ResizablePanel defaultSize={25} minSize={15} maxSize={40}>
        <div className="h-full border-r border-border">
          <div className="p-3 border-b border-border">
            <h3 className="text-sm font-medium">Changed Files</h3>
            <p className="text-xs text-muted-foreground mt-1">{parsedFiles.length} files changed</p>
          </div>
          <GitDiffFileTree
            files={parsedFiles}
            onFileSelect={handleFileSelect}
            selectedFileKey={selectedFileKey}
          />
        </div>
      </ResizablePanel>

      <ResizableHandle withHandle />

      {/* Right Panel - Diffs */}
      <ResizablePanel defaultSize={75}>
        <ScrollArea className="h-full overflow-auto p-4" ref={scrollAreaRef}>
          {/* Summary Stats */}
          {gitDiff && parsedFiles.length > 0 && (
            <div className="mb-4 text-sm text-muted-foreground">
              Showing {parsedFiles.length} changed files with{' '}
              <span className="text-status-success font-medium">
                {totalStats.totalAdditions} additions
              </span>{' '}
              and{' '}
              <span className="text-status-error font-medium">
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
                fileKey={fileKey}
                isExpanded={effectiveExpandedFiles.has(fileKey)}
                onToggle={() => toggleFileExpansion(fileKey)}
              />
            )
          })}
        </ScrollArea>
      </ResizablePanel>
    </ResizablePanelGroup>
  )
}
