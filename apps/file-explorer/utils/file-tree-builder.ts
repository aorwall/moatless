/**
 * Builds a hierarchical tree structure from a flat list of file paths.
 * Adapted from apps/sandbox/src/features/files/utils/file-tree-builder.ts
 * Self-contained — uses local type definitions.
 */

export interface FileTreeNode {
  name: string
  path: string
  type: 'file' | 'directory'
  isChanged: boolean
  gitStatus?: 'added' | 'modified' | 'deleted' | 'renamed'
  additions?: number
  deletions?: number
  isStaged?: boolean
  children?: FileTreeNode[]
}

interface GitFileInfo {
  status: 'added' | 'modified' | 'deleted' | 'renamed'
  additions: number
  deletions: number
  isStaged: boolean
}

export function buildFileTree(
  filePaths: string[],
  gitFileMap?: Map<string, GitFileInfo>,
): FileTreeNode[] {
  const root: FileTreeNode[] = []
  const nodeMap = new Map<string, FileTreeNode>()

  const sortedPaths = [...filePaths].sort()

  for (const filePath of sortedPaths) {
    processFilePath(filePath, root, nodeMap, gitFileMap)
  }

  propagateChangesToDirectories(root)
  return sortNodes(root)
}

function processFilePath(
  filePath: string,
  root: FileTreeNode[],
  nodeMap: Map<string, FileTreeNode>,
  gitFileMap?: Map<string, GitFileInfo>,
): void {
  const normalizedPath = filePath.endsWith('/') ? filePath.slice(0, -1) : filePath
  const parts = normalizedPath.split('/').filter(Boolean)
  let currentPath = ''
  let parentNode: FileTreeNode | null = null

  for (const [i, part] of parts.entries()) {
    currentPath = currentPath ? `${currentPath}/${part}` : part

    const isLastPart = i === parts.length - 1
    const isDirectory = isLastPart ? filePath.endsWith('/') : true

    let node = nodeMap.get(currentPath)

    if (!node) {
      node = createNode(part, currentPath, isDirectory, gitFileMap)
      nodeMap.set(currentPath, node)

      if (parentNode?.children) {
        parentNode.children.push(node)
      } else {
        root.push(node)
      }
    }

    parentNode = node
  }
}

function createNode(
  name: string,
  path: string,
  isDirectory: boolean,
  gitFileMap?: Map<string, GitFileInfo>,
): FileTreeNode {
  const gitInfo = gitFileMap?.get(path)

  return {
    name,
    path,
    type: isDirectory ? 'directory' : 'file',
    isChanged: !!gitInfo,
    ...(gitInfo && {
      gitStatus: gitInfo.status,
      additions: gitInfo.additions,
      deletions: gitInfo.deletions,
      isStaged: gitInfo.isStaged,
    }),
    children: isDirectory ? [] : undefined,
  }
}

function propagateChangesToDirectories(nodes: FileTreeNode[]): boolean {
  let hasChanges = false

  for (const node of nodes) {
    if (node.children && node.children.length > 0) {
      const childrenHaveChanges = propagateChangesToDirectories(node.children)
      if (childrenHaveChanges || node.isChanged) {
        node.isChanged = true
        hasChanges = true
      }
    } else if (node.isChanged) {
      hasChanges = true
    }
  }

  return hasChanges
}

function sortNodes(nodes: FileTreeNode[]): FileTreeNode[] {
  return nodes
    .sort((a, b) => {
      if (a.type !== b.type) {
        return a.type === 'directory' ? -1 : 1
      }
      return a.name.localeCompare(b.name)
    })
    .map((node) => ({
      ...node,
      children: node.children ? sortNodes(node.children) : undefined,
    }))
}
