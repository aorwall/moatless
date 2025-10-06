import type { FileTreeNode as FileTreeNodeType } from '@moatless/types'
import {
  ChevronRight,
  File,
  FileCheck,
  FilePlus,
  FileX,
  Filter,
  Folder,
  GitBranch,
} from 'lucide-react'
import { useMemo, useState } from 'react'
import { Button } from '@/lib/components/ui/button'
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
  const [showOnlyChanged, setShowOnlyChanged] = useState(false)
  const { data, isLoading, error } = useFileTreeWithChanges({ taskId })

  const handleFileSelect = (path: string) => {
    setSelectedPath(path)
    onFileSelect(path)
  }

  // Filter tree to show only changed files when toggle is on
  const filteredTree = useMemo(() => {
    if (!(data?.tree && showOnlyChanged)) return data?.tree || []

    const filterChangedNodes = (nodes: FileTreeNodeType[]): FileTreeNodeType[] => {
      return nodes.reduce((acc: FileTreeNodeType[], node) => {
        if (node.type === 'file' && node.isChanged) {
          acc.push(node)
        } else if (node.type === 'directory' && node.children) {
          const filteredChildren = filterChangedNodes(node.children)
          if (filteredChildren.length > 0) {
            acc.push({
              ...node,
              children: filteredChildren,
            })
          }
        }
        return acc
      }, [])
    }

    return filterChangedNodes(data.tree)
  }, [data?.tree, showOnlyChanged])

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
      <div className="p-3 border-b flex-shrink-0 space-y-2">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-medium">Files</h3>
          {hasChanges && (
            <Button
              variant={showOnlyChanged ? 'default' : 'outline'}
              size="sm"
              className="h-7 px-2"
              onClick={() => setShowOnlyChanged(!showOnlyChanged)}
              aria-label="Toggle changed files only"
            >
              <Filter className="h-3 w-3 mr-1" />
              Changed only
            </Button>
          )}
        </div>
        {hasChanges && (
          <div className="flex items-center gap-3 text-xs">
            <div className="flex items-center gap-1.5">
              <GitBranch className="h-3 w-3 text-muted-foreground" />
              <span className="text-muted-foreground">{data.gitSummary.totalFiles} files</span>
            </div>
            <div className="flex items-center gap-2">
              <span className="text-status-success">+{data.gitSummary.totalAdditions}</span>
              <span className="text-status-error">−{data.gitSummary.totalDeletions}</span>
            </div>
            {data.gitSummary.totalStaged > 0 && (
              <span className="text-muted-foreground">({data.gitSummary.totalStaged} staged)</span>
            )}
          </div>
        )}
      </div>
      <div className="flex-1 overflow-auto">
        <div className="min-w-max">
          {filteredTree.length === 0 && showOnlyChanged ? (
            <div className="p-4 text-sm text-muted-foreground text-center">
              No changed files to display
            </div>
          ) : (
            filteredTree.map((item: FileTreeNodeType) => (
              <TreeNode
                key={item.path}
                item={{ ...item, level: 0 }}
                selectedPath={selectedPath}
                onFileSelect={handleFileSelect}
              />
            ))
          )}
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
