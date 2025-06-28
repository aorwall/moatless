import { useMemo, useState } from 'react'
import { ErrorAlert } from '@/lib/components/ui/error-alert'
import { useFileList } from '../api/use-file-list'
import type { FileTreeNode } from '../types/file-tree'
import { buildTreeStructure } from '../utils/file-tree-utils'
import { FileTreeItem } from './file-tree-item'
import { FileTreeSkeleton } from './file-tree-skeleton'

interface FileTreeProps {
  taskId: string
  onFileSelect: (filePath: string) => void
}

export function FileTree({ taskId, onFileSelect }: FileTreeProps) {
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set())
  const [selectedPath, setSelectedPath] = useState<string | null>(null)
  const { data, isLoading, error } = useFileList({ taskId, recursive: true })

  const tree = useMemo(() => {
    if (!data?.files) return []
    return buildTreeStructure(data.files)
  }, [data?.files])

  const toggleExpanded = (path: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev)
      if (next.has(path)) {
        next.delete(path)
      } else {
        next.add(path)
      }
      return next
    })
  }

  const handleFileSelect = (path: string) => {
    setSelectedPath(path)
    onFileSelect(path)
  }

  const renderNode = (node: FileTreeNode, level = 0): React.ReactNode => {
    const isExpanded = expandedPaths.has(node.path)
    const isSelected = selectedPath === node.path

    return (
      <div key={node.path}>
        <FileTreeItem
          node={node}
          level={level}
          isExpanded={isExpanded}
          isSelected={isSelected}
          onToggleExpanded={() => toggleExpanded(node.path)}
          onSelect={() => handleFileSelect(node.path)}
        />
        {isExpanded && node.children && (
          <div>{node.children.map((child) => renderNode(child, level + 1))}</div>
        )}
      </div>
    )
  }

  if (error) {
    return <ErrorAlert error={error} />
  }

  if (isLoading) {
    return <FileTreeSkeleton />
  }

  if (!data?.files || data.files.length === 0) {
    return <div className="p-4 text-sm text-muted-foreground text-center">No files found</div>
  }

  return (
    <div className="file-tree h-full overflow-auto">
      <div className="min-w-max">
        {tree.map((node) => renderNode(node))}
      </div>
    </div>
  )
}
