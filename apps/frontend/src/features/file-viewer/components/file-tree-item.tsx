import type { FileTreeNode } from '@moatless/types'
import {
  ChevronDown,
  ChevronRight,
  File,
  FileCheck,
  FilePlus,
  FileX,
  Folder,
  FolderOpen,
} from 'lucide-react'
import { cn } from '@/lib/utils'

interface FileTreeItemProps {
  node: FileTreeNode
  level: number
  isExpanded: boolean
  isSelected: boolean
  onToggleExpanded: () => void
  onSelect: () => void
}

export function FileTreeItem({
  node,
  level,
  isExpanded,
  isSelected,
  onToggleExpanded,
  onSelect,
}: FileTreeItemProps) {
  const handleClick = () => {
    if (node.type === 'directory') {
      onToggleExpanded()
    } else {
      onSelect()
    }
  }

  const getIcon = () => {
    if (node.type === 'directory') {
      const FolderIcon = isExpanded ? FolderOpen : Folder
      return node.isChanged ? (
        <FolderIcon className="h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400" />
      ) : (
        <FolderIcon className="h-4 w-4 shrink-0 text-muted-foreground" />
      )
    }

    // File icons based on git status
    if (!node.isChanged) {
      return <File className="h-4 w-4 shrink-0 text-muted-foreground" />
    }

    switch (node.gitStatus) {
      case 'added':
        return <FilePlus className="h-4 w-4 shrink-0 text-green-600 dark:text-green-400" />
      case 'modified':
        return <FileCheck className="h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400" />
      case 'deleted':
        return <FileX className="h-4 w-4 shrink-0 text-red-600 dark:text-red-400" />
      default:
        return <File className="h-4 w-4 shrink-0 text-muted-foreground" />
    }
  }

  const ChevronIcon = isExpanded ? ChevronDown : ChevronRight

  return (
    <button
      type="button"
      className={cn(
        'flex items-center justify-between gap-1 px-2 py-1 hover:bg-accent cursor-pointer rounded-sm w-full text-left',
        isSelected && 'bg-accent',
      )}
      style={{ paddingLeft: `${level * 12 + 8}px` }}
      onClick={handleClick}
    >
      <div className="flex items-center gap-1">
        {node.type === 'directory' && (
          <ChevronIcon className="h-4 w-4 shrink-0 text-muted-foreground" />
        )}
        {node.type === 'file' && <div className="w-4" />}
        {getIcon()}
        <span className={cn('text-sm whitespace-nowrap', node.isChanged && 'font-medium')}>
          {node.name}
        </span>
      </div>
      {node.type === 'file' && node.isChanged && (
        <div className="flex items-center gap-1">
          {node.isStaged && (
            <span className="text-xs px-1.5 py-0.5 rounded-full bg-green-100 dark:bg-green-900 text-green-700 dark:text-green-300">
              staged
            </span>
          )}
          {node.additions !== undefined && node.deletions !== undefined && (
            <span className="text-xs text-muted-foreground">
              +{node.additions} -{node.deletions}
            </span>
          )}
        </div>
      )}
    </button>
  )
}
