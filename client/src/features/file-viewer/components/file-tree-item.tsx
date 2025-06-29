import { ChevronDown, ChevronRight, File, Folder, FolderOpen } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { FileTreeNode } from '../types/file-tree'

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

  const Icon = node.type === 'directory' ? (isExpanded ? FolderOpen : Folder) : File

  const ChevronIcon = isExpanded ? ChevronDown : ChevronRight

  return (
    <button
      type="button"
      className={cn(
        'flex items-center gap-1 px-2 py-1 hover:bg-accent cursor-pointer rounded-sm w-full text-left',
        isSelected && 'bg-accent',
      )}
      style={{ paddingLeft: `${level * 12 + 8}px` }}
      onClick={handleClick}
    >
      {node.type === 'directory' && (
        <ChevronIcon className="h-4 w-4 shrink-0 text-muted-foreground" />
      )}
      {node.type === 'file' && <div className="w-4" />}
      <Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
      <span className="text-sm whitespace-nowrap">{node.name}</span>
    </button>
  )
}
