import type { FileTreeRowDecorationRenderer } from '@pierre/trees'
import { FileTree as PierreFileTree, useFileTree as usePierreFileTree } from '@pierre/trees/react'
import { Filter, GitBranch, PanelLeftClose } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { useFileTree } from '../hooks/use-file-tree'
import { useFileViewState } from '../hooks/use-file-view-state'
import {
  buildGitStatusEntries,
  buildRowMetadata,
  collectChangedDirectoryPaths,
  filterChangedTree,
  flattenToPaths,
  type RowMetadata,
} from '../lib/flatten-tree'

interface FileTreeProps {
  taskId: string
  selectedPath: string | undefined
  onFileSelect: (filePath: string) => void
}

function decorationFromMetadata(metadata: RowMetadata | undefined): { text: string } | null {
  if (!metadata?.isChanged || metadata.type !== 'file') {
    return null
  }
  const additions = metadata.additions
  const deletions = metadata.deletions
  const hasCounts = additions !== undefined && deletions !== undefined
  if (!(hasCounts || metadata.isStaged)) {
    return null
  }

  const parts: string[] = []
  if (hasCounts) {
    parts.push(`+${additions} −${deletions}`)
  }
  if (metadata.isStaged) {
    parts.push('(staged)')
  }
  return { text: parts.join(' ') }
}

export function FileTree({ taskId, selectedPath, onFileSelect }: FileTreeProps) {
  const { setExplorerCollapsed } = useFileViewState()
  const { data, isLoading, error } = useFileTree(taskId)
  const [showOnlyChanged, setShowOnlyChanged] = useState(false)

  const tree = data?.tree ?? []
  const visibleTree = useMemo(
    () => (showOnlyChanged ? filterChangedTree(tree) : tree),
    [showOnlyChanged, tree],
  )

  const paths = useMemo(() => flattenToPaths(visibleTree), [visibleTree])
  const gitStatus = useMemo(() => buildGitStatusEntries(visibleTree), [visibleTree])
  const rowMetadata = useMemo(() => buildRowMetadata(tree), [tree])
  const initialExpandedPaths = useMemo(() => collectChangedDirectoryPaths(tree), [tree])

  const renderRowDecoration = useCallback<FileTreeRowDecorationRenderer>(
    ({ item }) => decorationFromMetadata(rowMetadata.get(item.path)),
    [rowMetadata],
  )

  const onSelectionChange = useCallback(
    (selected: readonly string[]) => {
      const first = selected[0]
      if (!first) {
        return
      }
      const meta = rowMetadata.get(first)
      if (meta?.type === 'directory') {
        return
      }
      onFileSelect(first)
    },
    [onFileSelect, rowMetadata],
  )

  const initialPaths = useRef(paths)
  const initialGitStatus = useRef(gitStatus)
  const initialExpansion = useRef(initialExpandedPaths)
  const initialSelection = useRef(selectedPath ? [selectedPath] : undefined)

  const { model } = usePierreFileTree({
    paths: initialPaths.current,
    gitStatus: initialGitStatus.current,
    onSelectionChange,
    ...(initialSelection.current !== undefined && {
      initialSelectedPaths: initialSelection.current,
    }),
    initialExpansion: 'closed',
    initialExpandedPaths: initialExpansion.current,
    renderRowDecoration,
  })

  useEffect(() => {
    if (paths === initialPaths.current) {
      return
    }
    model.resetPaths(paths, { initialExpandedPaths })
  }, [model, paths, initialExpandedPaths])

  useEffect(() => {
    if (gitStatus === initialGitStatus.current) {
      return
    }
    model.setGitStatus(gitStatus)
  }, [model, gitStatus])

  useEffect(() => {
    if (!selectedPath) {
      return
    }
    const current = model.getSelectedPaths()
    if (current.length === 1 && current[0] === selectedPath) {
      return
    }
    model.getItem(selectedPath)?.select()
  }, [model, selectedPath])

  if (error) {
    return <div className="p-4 text-sm text-destructive">Failed to load files: {error.message}</div>
  }

  if (isLoading) {
    return (
      <div className="p-4 space-y-2">
        {Array.from({ length: 8 }).map((_, i) => (
          <Skeleton
            // biome-ignore lint/suspicious/noArrayIndexKey: static skeleton list
            key={i}
            className="h-5"
            style={{ width: `${60 + Math.random() * 30}%` }}
          />
        ))}
      </div>
    )
  }

  if (!data?.tree || data.tree.length === 0) {
    return <div className="p-4 text-sm text-center text-muted-foreground">No files found</div>
  }

  const hasChanges = data.gitSummary.hasChanges

  return (
    <div className="h-full w-full flex flex-col">
      <div className="h-[42px] px-2 flex-shrink-0 flex items-center gap-2 border-b">
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="h-8 w-8 flex-shrink-0"
          onClick={() => setExplorerCollapsed(true)}
          aria-label="Collapse file tree"
        >
          <PanelLeftClose className="h-4 w-4" />
        </Button>
        {hasChanges && (
          <div className="flex items-center gap-3 ml-auto">
            <div className="flex items-center gap-3 text-xs">
              <div className="flex items-center gap-1.5">
                <GitBranch className="h-3 w-3 text-muted-foreground" />
                <span className="text-muted-foreground">{data.gitSummary.totalFiles} files</span>
              </div>
              <div className="flex items-center gap-2">
                <span style={{ color: 'var(--git-added)' }}>+{data.gitSummary.totalAdditions}</span>
                <span style={{ color: 'var(--git-deleted)' }}>
                  &minus;{data.gitSummary.totalDeletions}
                </span>
              </div>
            </div>
            <Button
              type="button"
              variant={showOnlyChanged ? 'secondary' : 'ghost'}
              size="icon"
              className="h-7 w-7 flex-shrink-0"
              onClick={() => setShowOnlyChanged((prev) => !prev)}
              aria-label={showOnlyChanged ? 'Show all files' : 'Show only changed files'}
              title={showOnlyChanged ? 'Show all files' : 'Show only changed files'}
            >
              <Filter className="h-4 w-4" />
            </Button>
          </div>
        )}
      </div>
      <div className="flex-1 min-h-0">
        {showOnlyChanged && paths.length === 0 ? (
          <div className="p-4 text-sm text-center text-muted-foreground">No changed files</div>
        ) : (
          <PierreFileTree model={model} style={{ height: '100%' }} />
        )}
      </div>
    </div>
  )
}
