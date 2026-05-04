import {
  ChevronDown,
  ChevronRight,
  File,
  FileCheck,
  FilePlus,
  FileX,
  Filter,
  Folder,
  FolderOpen,
  GitBranch,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import { Skeleton } from '@/lib/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { useFileTree } from '../hooks/use-file-tree'
import type { FileTreeNode } from '../types'

interface FileTreeProps {
  taskId: string
  selectedPath: string | undefined
  onFileSelect: (filePath: string) => void
}

export function FileTree({ taskId, selectedPath, onFileSelect }: FileTreeProps) {
  const [showOnlyChanged, setShowOnlyChanged] = useState(false)
  const { data, isLoading, error } = useFileTree(taskId)

  const tree = data?.tree ?? []
  const filteredTree = showOnlyChanged ? tree.filter((node) => node.isChanged) : tree

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
      <div className="p-3 flex-shrink-0 space-y-2 border-b">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-medium">Files</h3>
          {hasChanges && (
            <button
              type="button"
              onClick={() => setShowOnlyChanged(!showOnlyChanged)}
              className={cn(
                'h-7 px-2 text-xs rounded flex items-center gap-1 transition-colors',
                showOnlyChanged
                  ? 'bg-primary text-primary-foreground'
                  : 'border border-border hover:bg-accent',
              )}
            >
              <Filter className="h-3 w-3" />
              Changed
            </button>
          )}
        </div>
        {hasChanges && (
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
        )}
      </div>
      <div className="flex-1 overflow-auto">
        <div className="min-w-max py-1">
          {filteredTree.length === 0 && showOnlyChanged ? (
            <div className="p-4 text-sm text-center text-muted-foreground">No changed files</div>
          ) : (
            filteredTree.map((item) => (
              <TreeNode
                key={item.path}
                taskId={taskId}
                node={item}
                level={0}
                selectedPath={selectedPath}
                onFileSelect={onFileSelect}
                showOnlyChanged={showOnlyChanged}
              />
            ))
          )}
        </div>
      </div>
    </div>
  )
}

function TreeNode({
  taskId,
  node,
  level,
  selectedPath,
  onFileSelect,
  showOnlyChanged,
}: {
  taskId: string
  node: FileTreeNode
  level: number
  selectedPath: string | undefined
  onFileSelect: (path: string) => void
  showOnlyChanged: boolean
}) {
  const [isExpanded, setIsExpanded] = useState(node.isChanged)

  const handleClick = () => {
    if (node.type === 'directory') {
      setIsExpanded((prev) => !prev)
    } else {
      onFileSelect(node.path)
    }
  }

  const getFileIcon = () => {
    if (!node.isChanged) {
      return <File className="h-4 w-4 shrink-0" />
    }
    switch (node.gitStatus) {
      case 'added':
        return <FilePlus className="h-4 w-4 shrink-0" style={{ color: 'var(--git-added)' }} />
      case 'modified':
        return <FileCheck className="h-4 w-4 shrink-0" style={{ color: 'var(--git-modified)' }} />
      case 'deleted':
        return <FileX className="h-4 w-4 shrink-0" style={{ color: 'var(--git-deleted)' }} />
      default:
        return <File className="h-4 w-4 shrink-0" />
    }
  }

  const FolderIconComponent = isExpanded ? FolderOpen : Folder

  if (node.type === 'file') {
    return (
      <button
        type="button"
        onClick={handleClick}
        className={cn(
          'w-full flex items-center justify-between gap-1 px-2 py-1 text-sm rounded-sm hover:bg-accent transition-colors whitespace-nowrap',
          selectedPath === node.path && 'bg-accent',
        )}
        style={{ paddingLeft: `${level * 12 + 8}px` }}
      >
        <div className="flex items-center gap-1">
          <div className="w-4" />
          {getFileIcon()}
          <span className={cn(node.isChanged && 'font-medium')}>{node.name}</span>
        </div>
        {node.isChanged && (
          <div className="flex items-center gap-1">
            {node.isStaged && (
              <span
                className="text-xs px-1.5 py-0.5 rounded-full"
                style={{ background: 'rgba(34, 197, 94, 0.15)', color: 'var(--git-added)' }}
              >
                staged
              </span>
            )}
            {node.additions !== undefined && node.deletions !== undefined && (
              <div className="flex items-center gap-2 text-xs">
                <span style={{ color: 'var(--git-added)' }}>+{node.additions}</span>
                <span style={{ color: 'var(--git-deleted)' }}>&minus;{node.deletions}</span>
              </div>
            )}
          </div>
        )}
      </button>
    )
  }

  return (
    <div>
      <button
        type="button"
        onClick={handleClick}
        className="w-full flex items-center gap-1 px-2 py-1 text-sm rounded-sm hover:bg-accent transition-colors whitespace-nowrap"
        style={{ paddingLeft: `${level * 12 + 8}px` }}
      >
        {isExpanded ? (
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
        )}
        <FolderIconComponent
          className="h-4 w-4 shrink-0"
          style={node.isChanged ? { color: 'var(--git-modified)' } : undefined}
        />
        <span className={cn(node.isChanged && 'font-medium')}>{node.name}</span>
      </button>
      {isExpanded && (
        <LazyDirectoryChildren
          taskId={taskId}
          path={node.path}
          level={level + 1}
          selectedPath={selectedPath}
          onFileSelect={onFileSelect}
          showOnlyChanged={showOnlyChanged}
        />
      )}
    </div>
  )
}

function LazyDirectoryChildren({
  taskId,
  path,
  level,
  selectedPath,
  onFileSelect,
  showOnlyChanged,
}: {
  taskId: string
  path: string
  level: number
  selectedPath: string | undefined
  onFileSelect: (path: string) => void
  showOnlyChanged: boolean
}) {
  const { data, isLoading, error } = useFileTree(taskId, path)

  const visibleChildren = useMemo(() => {
    if (!data?.tree) {
      return undefined
    }
    if (!showOnlyChanged) {
      return data.tree
    }
    return data.tree.filter((child) => child.isChanged)
  }, [data?.tree, showOnlyChanged])

  if (error) {
    return (
      <div
        className="px-2 py-1 text-xs text-destructive"
        style={{ paddingLeft: `${level * 12 + 24}px` }}
      >
        Failed to load: {error.message}
      </div>
    )
  }

  if (isLoading || !visibleChildren) {
    return (
      <div className="space-y-1 py-1" style={{ paddingLeft: `${level * 12 + 24}px` }}>
        {Array.from({ length: 3 }).map((_, i) => (
          <Skeleton
            // biome-ignore lint/suspicious/noArrayIndexKey: static skeleton list
            key={i}
            className="h-4"
            style={{ width: `${50 + Math.random() * 30}%` }}
          />
        ))}
      </div>
    )
  }

  if (visibleChildren.length === 0) {
    return (
      <div
        className="px-2 py-1 text-xs text-muted-foreground"
        style={{ paddingLeft: `${level * 12 + 24}px` }}
      >
        Empty
      </div>
    )
  }

  return (
    <>
      {visibleChildren.map((child) => (
        <TreeNode
          key={child.path}
          taskId={taskId}
          node={child}
          level={level}
          selectedPath={selectedPath}
          onFileSelect={onFileSelect}
          showOnlyChanged={showOnlyChanged}
        />
      ))}
    </>
  )
}
