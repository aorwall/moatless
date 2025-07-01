import type { FileTreeNode } from '../types/file-tree'

export function buildTreeStructure(filePaths: string[]): FileTreeNode[] {
  const root: FileTreeNode[] = []
  const nodeMap = new Map<string, FileTreeNode>()

  // Sort paths to ensure parent directories come before their children
  const sortedPaths = [...filePaths].sort()

  for (const filePath of sortedPaths) {
    const parts = filePath.split('/').filter(Boolean)
    let currentPath = ''
    let parentNode: FileTreeNode[] = root

    for (let i = 0; i < parts.length; i++) {
      const part = parts[i]
      currentPath = currentPath ? `${currentPath}/${part}` : part

      const isLastPart = i === parts.length - 1
      const nodeType = isLastPart && !filePath.endsWith('/') ? 'file' : 'directory'

      let node = nodeMap.get(currentPath)

      if (!node) {
        node = {
          name: part,
          path: currentPath,
          type: nodeType,
          children: nodeType === 'directory' ? [] : undefined,
        }
        nodeMap.set(currentPath, node)
        parentNode.push(node)
      }

      if (node.children) {
        parentNode = node.children
      }
    }
  }

  return sortNodes(root)
}

function sortNodes(nodes: FileTreeNode[]): FileTreeNode[] {
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

export function getFileExtension(filePath: string): string {
  const parts = filePath.split('.')
  return parts.length > 1 ? parts[parts.length - 1] : ''
}

export function getLanguageFromPath(filePath: string): string {
  const ext = getFileExtension(filePath)
  const languageMap: Record<string, string> = {
    ts: 'typescript',
    tsx: 'typescript',
    js: 'javascript',
    jsx: 'javascript',
    py: 'python',
    rs: 'rust',
    go: 'go',
    java: 'java',
    c: 'c',
    cpp: 'cpp',
    cs: 'csharp',
    rb: 'ruby',
    php: 'php',
    swift: 'swift',
    kt: 'kotlin',
    scala: 'scala',
    r: 'r',
    dart: 'dart',
    lua: 'lua',
    pl: 'perl',
    sh: 'bash',
    bash: 'bash',
    zsh: 'bash',
    fish: 'bash',
    ps1: 'powershell',
    psm1: 'powershell',
    yaml: 'yaml',
    yml: 'yaml',
    json: 'json',
    xml: 'xml',
    html: 'html',
    htm: 'html',
    css: 'css',
    scss: 'scss',
    sass: 'sass',
    less: 'less',
    md: 'markdown',
    markdown: 'markdown',
    rst: 'restructuredtext',
    tex: 'latex',
    sql: 'sql',
    dockerfile: 'dockerfile',
    makefile: 'makefile',
    cmake: 'cmake',
    toml: 'toml',
    ini: 'ini',
    cfg: 'ini',
    conf: 'ini',
    vimrc: 'vim',
    vim: 'vim',
  }

  return languageMap[ext.toLowerCase()] || 'plaintext'
}
