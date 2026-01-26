import { mkdir, readdir, readFile, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import type { FileTreeWithChangesResponse } from '@moatless/types'
import type { SandboxConfig } from '../../../config/environment'
import { logger } from '../../../shared/logger'
import { getFileChangeInfo, getGitDiffDetailed } from '../../git/utils/git.utils'
import { buildFileTree } from '../utils/file-tree-builder'

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

export class FileService {
  constructor(private readonly config: SandboxConfig) {}

  private get workspacePath(): string {
    return this.config.workspaceDir
  }

  /**
   * Read a file from the workspace
   */
  async readFile(filePath: string): Promise<string> {
    const fullPath = path.join(this.workspacePath, filePath)

    try {
      return await readFile(fullPath, 'utf-8')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.error(
        { err: error as Error, taskId: this.config.taskId, filePath, errorMessage },
        'Failed to read file',
      )
      throw new Error(`Failed to read file ${filePath}: ${errorMessage}`)
    }
  }

  /**
   * Write a file to the workspace
   */
  async writeFile(filePath: string, content: string): Promise<void> {
    const fullPath = path.join(this.workspacePath, filePath)

    // Security check: ensure file is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(this.workspacePath)
    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: path outside workspace')
    }

    try {
      // Ensure directory exists
      await mkdir(path.dirname(fullPath), { recursive: true })
      await writeFile(fullPath, content, 'utf-8')

      logger.debug({ taskId: this.config.taskId, filePath }, 'File written successfully')
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error)
      logger.error(
        { err: error as Error, taskId: this.config.taskId, filePath, errorMessage },
        'Failed to write file',
      )
      throw new Error(`Failed to write file ${filePath}: ${errorMessage}`)
    }
  }

  /**
   * List files in the workspace
   */
  async listFiles(relativePath = '.', recursive = false): Promise<string[]> {
    const fullPath = path.join(this.workspacePath, relativePath)

    // Security check: ensure path is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(this.workspacePath)
    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: Path outside workspace')
    }

    try {
      const stats = await stat(fullPath)
      if (stats.isFile()) {
        return [relativePath]
      }

      const files: string[] = []

      async function walkDir(dir: string, basePath: string): Promise<void> {
        const entries = await readdir(dir, { withFileTypes: true })

        for (const entry of entries) {
          const entryFullPath = path.join(dir, entry.name)
          const entryRelativePath = path.join(basePath, entry.name)

          // Skip node_modules (typically too large to traverse)
          if (entry.name === 'node_modules') {
            continue
          }

          if (entry.isDirectory()) {
            files.push(`${entryRelativePath}/`)
            if (recursive) {
              await walkDir(entryFullPath, entryRelativePath)
            }
          } else {
            files.push(entryRelativePath)
          }
        }
      }

      await walkDir(fullPath, relativePath === '.' ? '' : relativePath)
      return files.sort()
    } catch (error) {
      if ((error as { code?: string }).code === 'ENOENT') {
        throw new Error(`Path not found: ${relativePath}`)
      }
      throw new Error(
        `Failed to list files: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  /**
   * Search for files in the workspace
   */
  async searchFiles(request: FileSearchRequest): Promise<FileSearchResult> {
    // Simple file search implementation
    // TODO: Could use ripgrep or other more efficient search if needed
    try {
      const allFiles = await this.listFiles('.', true)
      const results: Array<{ path: string; matches: number }> = []
      const query = request.caseSensitive ? request.query : request.query.toLowerCase()
      const maxResults = request.maxResults || 100

      for (const filePath of allFiles) {
        if (filePath.endsWith('/')) {
          continue // Skip directories
        }

        const searchPath = request.caseSensitive ? filePath : filePath.toLowerCase()

        if (searchPath.includes(query)) {
          results.push({ path: filePath, matches: 1 })
          if (results.length >= maxResults) {
            break
          }
        }
      }

      return {
        files: results,
        totalMatches: results.length,
      }
    } catch (error) {
      logger.error(
        { err: error as Error, taskId: this.config.taskId, query: request.query },
        'Failed to search files',
      )
      throw new Error(
        `Failed to search files: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  /**
   * Get file tree with git change information
   */
  async getFileTreeWithChanges(rootPath = '.'): Promise<FileTreeWithChangesResponse> {
    // Get all files recursively
    const allFiles = await this.listFiles(rootPath, true)

    // Get git diff information
    const gitDiff = await getGitDiffDetailed(this.workspacePath)

    // Create a map of file paths to git diff information for O(1) lookup
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

      if (file.isStaged) {
        totalStaged++
      } else {
        totalModified++
      }
    }

    // Build hierarchical tree structure with git information
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

  /**
   * Read a file with git change information
   */
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
    } catch (error) {
      logger.debug(
        {
          taskId: this.config.taskId,
          filePath,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to get file change info',
      )

      return {
        content,
        additions: 0,
        deletions: 0,
        isModified: false,
      }
    }
  }
}
