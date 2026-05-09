import type { GitStatusEntry } from '@pierre/trees'
import type { FileTreeNode } from '../types'

export interface RowMetadata {
  isStaged?: boolean
  additions?: number
  deletions?: number
  isChanged: boolean
  type: 'file' | 'directory'
}

function visit(nodes: readonly FileTreeNode[], visitor: (node: FileTreeNode) => void): void {
  for (const node of nodes) {
    visitor(node)
    if (node.children && node.children.length > 0) {
      visit(node.children, visitor)
    }
  }
}

export function flattenToPaths(tree: readonly FileTreeNode[]): string[] {
  const paths: string[] = []
  visit(tree, (node) => {
    paths.push(node.type === 'directory' ? `${node.path}/` : node.path)
  })
  return paths
}

export function buildGitStatusEntries(tree: readonly FileTreeNode[]): GitStatusEntry[] {
  const entries: GitStatusEntry[] = []
  visit(tree, (node) => {
    if (node.type !== 'file' || !node.gitStatus) {
      return
    }
    entries.push({
      path: node.path,
      status: node.gitStatus,
    })
  })
  return entries
}

export function buildRowMetadata(tree: readonly FileTreeNode[]): Map<string, RowMetadata> {
  const map = new Map<string, RowMetadata>()
  visit(tree, (node) => {
    map.set(node.path, {
      isStaged: node.isStaged,
      additions: node.additions,
      deletions: node.deletions,
      isChanged: node.isChanged,
      type: node.type,
    })
  })
  return map
}

export function collectChangedDirectoryPaths(tree: readonly FileTreeNode[]): string[] {
  const dirs = new Set<string>()
  visit(tree, (node) => {
    if (!node.isChanged) {
      return
    }
    const segments = node.path.split('/')
    for (let i = 1; i < segments.length; i++) {
      dirs.add(segments.slice(0, i).join('/'))
    }
  })
  return Array.from(dirs)
}

export function filterChangedTree(tree: readonly FileTreeNode[]): FileTreeNode[] {
  const result: FileTreeNode[] = []
  for (const node of tree) {
    if (!node.isChanged) {
      continue
    }
    if (node.type === 'directory' && node.children) {
      const filteredChildren = filterChangedTree(node.children)
      result.push({ ...node, children: filteredChildren })
    } else {
      result.push(node)
    }
  }
  return result
}
