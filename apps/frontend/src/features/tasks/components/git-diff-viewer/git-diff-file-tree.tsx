import { FileTreeItem } from '@/features/file-viewer'
import React, { useState } from 'react'
import type { FileTreeNode } from '@/features/file-viewer'
import { buildTreeStructure } from '@/features/file-viewer'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import type { FileData } from 'react-diff-view'

interface GitDiffFileTreeProps {
  files: FileData[]
  onFileSelect: (fileKey: string) => void
  selectedFileKey?: string
}

export function GitDiffFileTree({ files, onFileSelect, selectedFileKey }: GitDiffFileTreeProps) {
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set())

  // Extract file paths from the parsed diff files
  const filePaths = files.map((file) => {
    const path = file.newPath || file.oldPath || ''
    return path
  }).filter(Boolean)

  // Build tree structure
  const tree = buildTreeStructure(filePaths)

  // Create a map for quick lookup of file keys by path
  const fileKeyMap = new Map<string, string>()
  files.forEach((file) => {
    const path = file.newPath || file.oldPath || ''
    const fileKey = `${file.oldPath || file.oldRevision}-${file.newPath || file.newRevision}`
    if (path) {
      fileKeyMap.set(path, fileKey)
    }
  })

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

  const renderNode = (node: FileTreeNode, level = 0): React.JSX.Element => {
    const isExpanded = expandedPaths.has(node.path)
    const fileKey = fileKeyMap.get(node.path)
    const isSelected = fileKey === selectedFileKey

    return (
      <div key={node.path}>
        <FileTreeItem
          node={node}
          level={level}
          isExpanded={isExpanded}
          isSelected={isSelected}
          onToggleExpanded={() => toggleExpanded(node.path)}
          onSelect={() => {
            if (fileKey) {
              onFileSelect(fileKey)
            }
          }}
        />
        {isExpanded && node.children && (
          <div>
            {node.children.map((child) => renderNode(child, level + 1))}
          </div>
        )}
      </div>
    )
  }

  // Auto-expand all directories initially
  if (expandedPaths.size === 0 && tree.length > 0) {
    const allDirectories = new Set<string>()
    const collectDirectories = (nodes: FileTreeNode[]) => {
      nodes.forEach((node) => {
        if (node.type === 'directory') {
          allDirectories.add(node.path)
          if (node.children) {
            collectDirectories(node.children)
          }
        }
      })
    }
    collectDirectories(tree)
    setExpandedPaths(allDirectories)
  }

  return (
    <ScrollArea className="h-full">
      <div className="p-2">
        {tree.map((node) => renderNode(node))}
      </div>
    </ScrollArea>
  )
}