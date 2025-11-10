import type { FileTreeNodeType } from '@moatless/types'

interface GitFileInfo {
  status: 'added' | 'modified' | 'deleted' | 'renamed'
  additions: number
  deletions: number
  isStaged: boolean
}

/**
 * Builds a hierarchical tree structure from a flat list of file paths
 *
 * @param filePaths - Flat array of file paths (directories end with '/')
 * @param gitFileMap - Optional map of file paths to git change information
 * @returns Nested tree structure with directories and files
 *
 * @example
 * const paths = ['src/', 'src/index.ts', 'README.md']
 * const tree = buildFileTree(paths)
 * // Returns:
 * // [
 * //   { name: 'README.md', path: 'README.md', type: 'file', ... },
 * //   { name: 'src', path: 'src', type: 'directory', children: [...], ... }
 * // ]
 */
export function buildFileTree(
  filePaths: string[],
  gitFileMap?: Map<string, GitFileInfo>,
): FileTreeNodeType[] {
  const root: FileTreeNodeType[] = []
  const nodeMap = new Map<string, FileTreeNodeType>()

  // Sort paths to ensure parent directories come before their children
  const sortedPaths = [...filePaths].sort()

  // First pass: Create all nodes
  for (const filePath of sortedPaths) {
    processFilePath(filePath, root, nodeMap, gitFileMap)
  }

  // Second pass: Propagate git changes up to parent directories
  propagateChangesToDirectories(root)

  return sortNodes(root)
}

/**
 * Processes a single file path and adds it to the tree structure
 */
function processFilePath(
  filePath: string,
  _root: FileTreeNodeType[],
  nodeMap: Map<string, FileTreeNodeType>,
  gitFileMap?: Map<string, GitFileInfo>,
): void {
  // Remove trailing slash from directories for consistent path handling
  const normalizedPath = filePath.endsWith('/') ? filePath.slice(0, -1) : filePath
  const parts = normalizedPath.split('/').filter(Boolean)
  let currentPath = ''

  for (let i = 0; i < parts.length; i++) {
    const part = parts[i]!
    currentPath = currentPath ? `${currentPath}/${part}` : part

    const isLastPart = i === parts.length - 1
    const isDirectory = isLastPart ? filePath.endsWith('/') : true

    let node = nodeMap.get(currentPath)

    if (!node) {
      node = createNode(part, currentPath, isDirectory, gitFileMap)
      nodeMap.set(currentPath, node)
    }
  }
}

/**
 * Creates a new tree node with git information if available
 */
function createNode(
  name: string,
  path: string,
  isDirectory: boolean,
  gitFileMap?: Map<string, GitFileInfo>,
): FileTreeNodeType {
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

/**
 * Recursively marks directories as changed if any of their children are changed
 */
function propagateChangesToDirectories(nodes: FileTreeNodeType[]): boolean {
  let hasChanges = false

  for (const node of nodes) {
    if (node.children && node.children.length > 0) {
      // Recursively check children
      const childrenHaveChanges = propagateChangesToDirectories(node.children)

      // Mark directory as changed if children have changes
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

/**
 * Sorts nodes recursively: directories first (alphabetically), then files (alphabetically)
 */
function sortNodes(nodes: FileTreeNodeType[]): FileTreeNodeType[] {
  return nodes
    .sort((a, b) => {
      // Directories first, then files
      if (a.type !== b.type) {
        return a.type === 'directory' ? -1 : 1
      }
      // Alphabetical order within same type
      return a.name.localeCompare(b.name)
    })
    .map((node) => ({
      ...node,
      children: node.children ? sortNodes(node.children) : undefined,
    }))
}
