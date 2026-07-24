import type { FileTreeRowDecorationRenderer } from '@pierre/trees'
import { FileTree as PierreFileTree, useFileTree as usePierreFileTree } from '@pierre/trees/react'
import { Filter, GitBranch, PanelLeftClose } from 'lucide-react'
import { useEffect, useState } from 'react'
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

const FILE_TREE_SKELETON_WIDTHS = [72, 86, 64, 79, 90, 68, 83, 75] as const

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

  const tree = data?.tree
  const visibleTree = (() => {
    const currentTree = tree ?? []
    return showOnlyChanged ? filterChangedTree(currentTree) : currentTree
  })()

  const paths = flattenToPaths(visibleTree)
  const gitStatus = buildGitStatusEntries(visibleTree)
  const rowMetadata = buildRowMetadata(tree ?? [])
  const initialExpandedPaths = collectChangedDirectoryPaths(tree ?? [])

  const renderRowDecoration: FileTreeRowDecorationRenderer = ({ item }) =>
    decorationFromMetadata(rowMetadata.get(item.path))

  const onSelectionChange = (selected: readonly string[]) => {
    const first = selected[0]
    if (!first) {
      return
    }
    const lookupPath = first.endsWith('/') ? first.slice(0, -1) : first
    const meta = rowMetadata.get(lookupPath)
    if (meta?.type === 'directory') {
      return
    }
    onFileSelect(first)
  }

  const [initialPaths] = useState(paths)
  const [initialGitStatus] = useState(gitStatus)
  const [initialExpansion] = useState(initialExpandedPaths)
  const [initialSelection] = useState(selectedPath ? [selectedPath] : undefined)

  const { model } = usePierreFileTree({
    paths: initialPaths,
    gitStatus: initialGitStatus,
    onSelectionChange,
    ...(initialSelection !== undefined && {
      initialSelectedPaths: initialSelection,
    }),
    initialExpansion: 'closed',
    initialExpandedPaths: initialExpansion,
    renderRowDecoration,
  })

  useEffect(() => {
    if (paths === initialPaths) {
      return
    }
    model.resetPaths(paths, { initialExpandedPaths })
  }, [model, paths, initialPaths, initialExpandedPaths])

  useEffect(() => {
    if (gitStatus === initialGitStatus) {
      return
    }
    model.setGitStatus(gitStatus)
  }, [model, gitStatus, initialGitStatus])

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
        {FILE_TREE_SKELETON_WIDTHS.map((width) => (
          <Skeleton key={width} className="h-5" style={{ width: `${width}%` }} />
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
