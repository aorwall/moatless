/**
 * File service for the file explorer server.
 * Adapted from apps/sandbox/src/features/files/services/file.service.ts
 * Self-contained — reads directly from disk, no sandbox dependency.
 */
import { mkdir, readdir, readFile, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { getFileChangeInfo, getGitDiffDetailed } from '../utils/git.utils'
import { type FileTreeNode, buildFileTree } from '../utils/file-tree-builder'

export interface FileSearchRequest {
  query: string
  path?: string
  caseSensitive?: boolean
  maxResults?: number
}

export interface FileSearchResult {
  files: Array<{ path: string; matches: number }>
  totalMatches: number
}

export interface GitSummary {
  totalFiles: number
  totalAdditions: number
  totalDeletions: number
  totalStaged: number
  totalModified: number
  compareBase: string
  hasChanges: boolean
}

export interface FileTreeWithChangesResponse {
  tree: FileTreeNode[]
  gitSummary: GitSummary
}

export class FileService {
  constructor(private readonly workspacePath: string) {}

  async readFile(filePath: string): Promise<string> {
    const fullPath = path.join(this.workspacePath, filePath)
    return await readFile(fullPath, 'utf-8')
  }

  async writeFile(filePath: string, content: string): Promise<void> {
    const fullPath = path.join(this.workspacePath, filePath)

    // Security check: ensure file is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(this.workspacePath)
    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: path outside workspace')
    }

    await mkdir(path.dirname(fullPath), { recursive: true })
    await writeFile(fullPath, content, 'utf-8')
  }

  async listFiles(relativePath = '.', recursive = false): Promise<string[]> {
    const fullPath = path.join(this.workspacePath, relativePath)

    // Security check
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(this.workspacePath)
    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: Path outside workspace')
    }

    const stats = await stat(fullPath)
    if (stats.isFile()) return [relativePath]

    const files: string[] = []

    async function walkDir(dir: string, basePath: string): Promise<void> {
      const entries = await readdir(dir, { withFileTypes: true })

      for (const entry of entries) {
        const entryFullPath = path.join(dir, entry.name)
        const entryRelativePath = path.join(basePath, entry.name)

        if (entry.name === 'node_modules' || entry.name === '.git') continue

        if (entry.isDirectory()) {
          files.push(`${entryRelativePath}/`)
          if (recursive) await walkDir(entryFullPath, entryRelativePath)
        } else {
          files.push(entryRelativePath)
        }
      }
    }

    await walkDir(fullPath, relativePath === '.' ? '' : relativePath)
    return files.sort()
  }

  async searchFiles(request: FileSearchRequest): Promise<FileSearchResult> {
    const allFiles = await this.listFiles('.', true)
    const results: Array<{ path: string; matches: number }> = []
    const query = request.caseSensitive ? request.query : request.query.toLowerCase()
    const maxResults = request.maxResults || 100

    for (const filePath of allFiles) {
      if (filePath.endsWith('/')) continue
      const searchPath = request.caseSensitive ? filePath : filePath.toLowerCase()
      if (searchPath.includes(query)) {
        results.push({ path: filePath, matches: 1 })
        if (results.length >= maxResults) break
      }
    }

    return { files: results, totalMatches: results.length }
  }

  async getFileTreeWithChanges(rootPath = '.'): Promise<FileTreeWithChangesResponse> {
    const allFiles = await this.listFiles(rootPath, true)
    const gitDiff = await getGitDiffDetailed(this.workspacePath)

    const gitFileMap = new Map<
      string,
      {
        status: 'added' | 'modified' | 'deleted' | 'renamed'
        additions: number
        deletions: number
        isStaged: boolean
      }
    >()

    let totalStaged = 0
    let totalModified = 0

    for (const file of gitDiff.files) {
      gitFileMap.set(file.filename, {
        status: file.status,
        additions: file.additions,
        deletions: file.deletions,
        isStaged: file.isStaged,
      })
      if (file.isStaged) totalStaged++
      else totalModified++
    }

    const tree = buildFileTree(allFiles, gitFileMap)

    return {
      tree,
      gitSummary: {
        totalFiles: gitDiff.files.length,
        totalAdditions: gitDiff.totalAdditions,
        totalDeletions: gitDiff.totalDeletions,
        totalStaged,
        totalModified,
        compareBase: gitDiff.compareBase || 'HEAD',
        hasChanges: gitDiff.files.length > 0,
      },
    }
  }

  async readFileWithChanges(filePath: string): Promise<{
    content: string
    oldContent?: string
    additions?: number
    deletions?: number
    isModified?: boolean
  }> {
    const content = await this.readFile(filePath)

    try {
      const changeInfo = await getFileChangeInfo(this.workspacePath, filePath)
      return {
        content,
        ...(changeInfo.oldContent && { oldContent: changeInfo.oldContent }),
        ...(changeInfo.additions !== undefined && { additions: changeInfo.additions }),
        ...(changeInfo.deletions !== undefined && { deletions: changeInfo.deletions }),
        ...(changeInfo.isModified !== undefined && { isModified: changeInfo.isModified }),
      }
    } catch {
      return { content, additions: 0, deletions: 0, isModified: false }
    }
  }
}
