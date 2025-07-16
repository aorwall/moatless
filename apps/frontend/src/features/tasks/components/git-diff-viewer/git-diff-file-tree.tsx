import type { FileTreeNode } from '@moatless/types'
import React from 'react'
import type { FileData } from 'react-diff-view'
import { buildTreeStructure, FileTreeItem } from '@/features/file-viewer'
import { useGitDiffState } from '@/features/tasks/contexts/task-ui-provider'
import { ScrollArea } from '@/lib/components/ui/scroll-area'

interface GitDiffFileTreeProps {
  files: FileData[]
  onFileSelect: (fileKey: string) => void
  selectedFileKey?: string
}

export function GitDiffFileTree({ files, onFileSelect, selectedFileKey }: GitDiffFileTreeProps) {
  const { gitDiffState, updateGitDiffState } = useGitDiffState()
  const { expandedPaths } = gitDiffState

  // Extract file paths from the parsed diff files
  const filePaths = files
    .map((file) => {
      const path = file.newPath || file.oldPath || ''
      return path
    })
    .filter(Boolean)

  // Build tree structure and add isChanged property
  const localTree = buildTreeStructure(filePaths)

  // Convert local FileTreeNode to @moatless/types FileTreeNode by adding isChanged
  const addIsChanged = (nodes: any[]): FileTreeNode[] => {
    return nodes.map((node) => ({
      name: node.name,
      path: node.path,
      type: node.type,
      isChanged: true, // All files in git diff are changed
      children: node.children ? addIsChanged(node.children) : undefined,
    }))
  }

  const tree = addIsChanged(localTree as any[])

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
    const next = new Set(expandedPaths)
    if (next.has(path)) {
      next.delete(path)
    } else {
      next.add(path)
    }
    updateGitDiffState({ expandedPaths: next })
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
          <div>{node.children.map((child) => renderNode(child, level + 1))}</div>
        )}
      </div>
    )
  }

  // Auto-expand all directories initially
  React.useEffect(() => {
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
      updateGitDiffState({ expandedPaths: allDirectories })
    }
  }, [tree, expandedPaths.size, updateGitDiffState])

  return (
    <ScrollArea className="h-full">
      <div className="p-2">{tree.map((node) => renderNode(node))}</div>
    </ScrollArea>
  )
}
