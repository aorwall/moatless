import type { FileTreeNode as FileTreeNodeType } from '@moatless/types'
import { ChevronRight, File, FileCheck, FilePlus, FileX, Folder, GitBranch } from 'lucide-react'
import { useState } from 'react'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/lib/components/ui/collapsible'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { cn } from '@/lib/utils'
import { useFileTreeWithChanges } from '../api/use-file-tree-with-changes'
import { FileTreeSkeleton } from './file-tree-skeleton'

interface FileTreeProps {
  taskId: string
  onFileSelect: (filePath: string) => void
}

export function FileTree({ taskId, onFileSelect }: FileTreeProps) {
  const [selectedPath, setSelectedPath] = useState<string | null>(null)
  const { data, isLoading, error } = useFileTreeWithChanges({ taskId })

  const handleFileSelect = (path: string) => {
    setSelectedPath(path)
    onFileSelect(path)
  }

  if (error) {
    return <ErrorAlert error={error} />
  }

  if (isLoading) {
    return <FileTreeSkeleton />
  }

  if (!data?.tree || data.tree.length === 0) {
    return <div className="p-4 text-sm text-muted-foreground text-center">No files found</div>
  }

  const hasChanges = data.gitSummary.hasChanges

  return (
    <div className="h-full w-full flex flex-col">
      <div className="p-3 border-b flex-shrink-0">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-medium">Files</h3>
          {hasChanges && (
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <GitBranch className="h-3 w-3" />
              <span>
                {data.gitSummary.totalFiles} changed
                {data.gitSummary.totalStaged > 0 && ` (${data.gitSummary.totalStaged} staged)`}
              </span>
            </div>
          )}
        </div>
      </div>
      <div className="flex-1 overflow-auto">
        <div className="min-w-max">
          {data.tree.map((item: FileTreeNodeType) => (
            <TreeNode
              key={item.path}
              item={{ ...item, level: 0 }}
              selectedPath={selectedPath}
              onFileSelect={handleFileSelect}
            />
          ))}
        </div>
      </div>
    </div>
  )
}

function TreeNode({
  item,
  selectedPath,
  onFileSelect,
}: {
  item: FileTreeNodeType & { level?: number }
  selectedPath: string | null
  onFileSelect: (path: string) => void
}) {
  const getFileIcon = () => {
    if (!item.isChanged) return <File className="h-4 w-4 flex-shrink-0" />

    switch (item.gitStatus) {
      case 'added':
        return <FilePlus className="h-4 w-4 flex-shrink-0 text-green-600 dark:text-green-400" />
      case 'modified':
        return <FileCheck className="h-4 w-4 flex-shrink-0 text-amber-600 dark:text-amber-400" />
      case 'deleted':
        return <FileX className="h-4 w-4 flex-shrink-0 text-red-600 dark:text-red-400" />
      default:
        return <File className="h-4 w-4 flex-shrink-0" />
    }
  }

  const getStatusBadge = () => {
    if (!item.isChanged) return null

    return (
      <div className="flex items-center gap-1">
        {item.isStaged && (
          <span className="text-xs px-1.5 py-0.5 rounded-full bg-green-100 dark:bg-green-900 text-green-700 dark:text-green-300">
            staged
          </span>
        )}
        {item.additions !== undefined && item.deletions !== undefined && (
          <div className="flex items-center gap-2 text-xs">
            <span className="text-status-success">+{item.additions}</span>
            <span className="text-status-error">−{item.deletions}</span>
          </div>
        )}
      </div>
    )
  }

  if (item.type === 'file') {
    return (
      <div>
        <button
          type="button"
          onClick={() => onFileSelect(item.path)}
          className={cn(
            'w-full flex items-center justify-between gap-2 px-2 py-1 text-sm rounded-sm hover:bg-accent transition-colors whitespace-nowrap',
            selectedPath === item.path && 'bg-accent',
          )}
          style={{ paddingLeft: `${(item.level || 0) * 12 + 8}px` }}
        >
          <div className="flex items-center gap-1">
            <div className="w-4" />
            {getFileIcon()}
            <span className={cn('text-left text-foreground', item.isChanged && 'font-medium')}>
              {item.name}
            </span>
          </div>
          {getStatusBadge()}
        </button>
      </div>
    )
  }

  const folderIcon = item.isChanged ? (
    <Folder className="h-4 w-4 flex-shrink-0 text-amber-600 dark:text-amber-400" />
  ) : (
    <Folder className="h-4 w-4 flex-shrink-0" />
  )

  return (
    <div>
      <Collapsible defaultOpen={item.isChanged}>
        <CollapsibleTrigger asChild>
          <button
            type="button"
            className="w-full flex items-center gap-1 px-2 py-1 text-sm rounded-sm hover:bg-accent transition-colors whitespace-nowrap"
            style={{ paddingLeft: `${(item.level || 0) * 12 + 8}px` }}
          >
            <ChevronRight className="h-4 w-4 flex-shrink-0 transition-transform data-[state=open]:rotate-90" />
            {folderIcon}
            <span className={cn('text-left text-foreground', item.isChanged && 'font-medium')}>
              {item.name}
            </span>
          </button>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <div>
            {item.children?.map((subItem: FileTreeNodeType) => (
              <TreeNode
                key={subItem.path}
                item={{ ...subItem, level: (item.level || 0) + 1 }}
                selectedPath={selectedPath}
                onFileSelect={onFileSelect}
              />
            ))}
          </div>
        </CollapsibleContent>
      </Collapsible>
    </div>
  )
}
