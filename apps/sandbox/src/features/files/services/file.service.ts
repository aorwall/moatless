import { readdir, stat } from 'node:fs/promises'
import path from 'node:path'
import type { SandboxConfig } from '../../../config/environment'
import { logger } from '../../../shared/logger'

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
   * List files in the workspace (used internally by searchFiles)
   */
  private async listFiles(relativePath = '.', recursive = false): Promise<string[]> {
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
}
