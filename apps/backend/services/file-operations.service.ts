import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { GitChanges, GitStatus } from '@moatless/types'
import { WorkspaceError } from '@moatless/types'
import {
  type GitDiffResponse,
  getGitDiffDetailed,
  hasUncommittedChanges,
  isGitRepository,
  searchFilesInDirectoryNew,
} from '../utils/index.js'
import { createTaskLogger } from '../utils/logger.js'

export interface FileSearchRequest {
  query: string
  limit?: number
}

export interface FileSearchResult {
  path: string
  absolutePath: string
  score: number
}

export interface FileSearchResponse {
  results: FileSearchResult[]
  totalFound: number
  query: string
}

/**
 * Service responsible for all file system and git operations within a workspace.
 * This includes reading/writing files, listing directories, and git operations.
 */
export class FileOperations {
  private logger

  constructor(
    private readonly taskId: string,
    private readonly workspacePath: string,
    private readonly originalBranch?: string,
  ) {
    this.logger = createTaskLogger(this.taskId)
  }

  /**
   * Check if a file or directory exists
   */
  async pathExists(filePath: string): Promise<boolean> {
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }

  /**
   * Check if the workspace path is valid and accessible
   */
  async isWorkspacePathValid(): Promise<boolean> {
    if (!this.workspacePath) return false

    try {
      await fs.access(this.workspacePath)

      // For git worktrees, also check if it's still a valid git repository
      return await isGitRepository(this.workspacePath)
    } catch {
      return false
    }
  }

  /**
   * Check if the workspace path exists and is a valid git worktree
   */
  async isValidWorktree(): Promise<boolean> {
    if (!this.workspacePath) return false

    try {
      await fs.access(this.workspacePath)
      // Check if it's a git worktree by looking for .git file (not directory)
      const gitPath = path.join(this.workspacePath, '.git')
      const gitStat = await fs.stat(gitPath)
      return gitStat.isFile() // Worktrees have .git file, main repos have .git directory
    } catch {
      return false
    }
  }

  /**
   * List files in the workspace
   */
  async listFiles(relativePath = '.', recursive = false): Promise<string[]> {
    const fullPath = path.join(this.workspacePath, relativePath)

    // Security: Ensure path is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(this.workspacePath)
    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: Path outside workspace')
    }

    try {
      const stats = await fs.stat(fullPath)
      if (stats.isFile()) {
        return [relativePath]
      }

      const files: string[] = []

      async function walkDir(dir: string, baseDir: string, basePath: string): Promise<void> {
        const entries = await fs.readdir(dir, { withFileTypes: true })

        for (const entry of entries) {
          const fullPath = path.join(dir, entry.name)
          const relativePath = path.join(basePath, entry.name)

          // Skip hidden files and common ignore patterns
          if (entry.name.startsWith('.') || entry.name === 'node_modules') {
            continue
          }

          if (entry.isDirectory()) {
            files.push(`${relativePath}/`)
            if (recursive) {
              await walkDir(fullPath, baseDir, relativePath)
            }
          } else {
            files.push(relativePath)
          }
        }
      }

      await walkDir(fullPath, this.workspacePath, relativePath === '.' ? '' : relativePath)
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
   * Read a file from the workspace
   */
  async readFile(filePath: string): Promise<string> {
    const fullPath = path.join(this.workspacePath, filePath)

    // Security check: ensure file is within workspace
    const resolvedPath = path.resolve(fullPath)
    const resolvedWorkspace = path.resolve(this.workspacePath)

    if (!resolvedPath.startsWith(resolvedWorkspace)) {
      throw new Error('Access denied: path outside workspace')
    }

    try {
      // Check file exists and is readable
      const stats = await fs.stat(fullPath)
      if (!stats.isFile()) {
        throw new Error('Path is not a file')
      }

      // Read file with size limit (10MB)
      const MAX_FILE_SIZE = 10 * 1024 * 1024
      if (stats.size > MAX_FILE_SIZE) {
        throw new Error('File too large to read')
      }

      return await fs.readFile(fullPath, 'utf-8')
    } catch (error) {
      if ((error as { code?: string }).code === 'ENOENT') {
        throw new Error(`File not found: ${filePath}`)
      }
      if (error instanceof Error && error.message.includes('Access denied')) {
        throw error
      }
      if (error instanceof Error && error.message.includes('File too large')) {
        throw error
      }
      if (error instanceof Error && error.message.includes('Path is not a file')) {
        throw error
      }
      throw new Error(
        `Failed to read file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  /**
   * Read a file from the workspace with change information
   */
  async readFileWithChanges(filePath: string): Promise<{
    content: string
    oldContent?: string
    additions?: number
    deletions?: number
    isModified?: boolean
  }> {
    // First read the current content
    const content = await this.readFile(filePath)

    try {
      // Get the file change information
      const { getFileChangeInfo } = await import('../utils/git-utils.js')
      const changeInfo = await getFileChangeInfo(this.workspacePath, filePath, this.originalBranch)

      // Only include change information if the file is actually modified
      if (changeInfo.isModified) {
        return {
          content,
          oldContent: changeInfo.oldContent || undefined,
          additions: changeInfo.additions,
          deletions: changeInfo.deletions,
          isModified: changeInfo.isModified,
        }
      }
    } catch (error) {
      // If we can't get git info, just return the content without change info
      this.logger.debug('Failed to get file change info', {
        filePath,
        error: error instanceof Error ? error.message : String(error),
      })
    }

    // Return just the content if no changes or error getting change info
    return { content }
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
      await fs.mkdir(path.dirname(fullPath), { recursive: true })
      await fs.writeFile(fullPath, content, 'utf-8')
    } catch (error) {
      throw new Error(
        `Failed to write file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  /**
   * Search for files in the workspace
   */
  async searchFiles(request: FileSearchRequest): Promise<FileSearchResponse> {
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      throw new WorkspaceError(
        `Workspace path does not exist: ${this.workspacePath}`,
        this.taskId,
        400,
      )
    }

    return await searchFilesInDirectoryNew(this.workspacePath, request)
  }

  /**
   * Get git diff for the workspace
   */
  async getGitDiff(): Promise<GitDiffResponse> {
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      throw new WorkspaceError(
        `Workspace path does not exist: ${this.workspacePath}`,
        this.taskId,
        400,
      )
    }

    return getGitDiffDetailed(this.workspacePath, this.originalBranch)
  }

  /**
   * Check if the workspace has uncommitted git changes
   */
  async hasUncommittedChanges(): Promise<boolean> {
    // If workspace doesn't exist or is not set up, no changes
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      return false
    }

    return hasUncommittedChanges(this.workspacePath)
  }

  /**
   * Get git status with additions, deletions, modifications
   */
  async getGitStatus(): Promise<GitStatus> {
    // If workspace doesn't exist or is not set up, return defaults
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      this.logger.warn('Workspace path does not exist when getting git status', {
        workspacePath: this.workspacePath,
      })
      return {
        currentBranch: 'unknown',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        status: 'error',
        commitsAhead: 0,
        commitsBehind: 0,
        originExists: false,
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }

    try {
      // Import the functions from git-utils
      const { getOriginSyncStatus, checkForConflicts, getGitDiffDetailed } = await import(
        '../utils/git-utils.js'
      )
      const [syncStatus, hasConflicts, gitDiff] = await Promise.all([
        getOriginSyncStatus(this.workspacePath, this.originalBranch),
        checkForConflicts(this.workspacePath, this.originalBranch),
        getGitDiffDetailed(this.workspacePath, this.originalBranch),
      ])

      // Count file changes by type
      let additions = 0
      let deletions = 0
      let modifications = 0

      for (const file of gitDiff.files) {
        if (file.status === 'added') {
          additions++
        } else if (file.status === 'deleted') {
          deletions++
        } else if (file.status === 'modified') {
          modifications++
        }
      }

      return {
        currentBranch: syncStatus.currentBranch,
        baseBranch: syncStatus.baseBranch,
        hasUncommittedChanges: syncStatus.hasUncommittedChanges,
        hasConflicts,
        status: syncStatus.status,
        commitsAhead: syncStatus.commitsAhead,
        commitsBehind: syncStatus.commitsBehind,
        originExists: syncStatus.originExists,
        additions,
        deletions,
        modifications,
      }
    } catch (_error) {
      // Fall back to defaults on error
      return {
        currentBranch: 'unknown',
        baseBranch: 'main',
        hasUncommittedChanges: false,
        hasConflicts: false,
        status: 'error',
        commitsAhead: 0,
        commitsBehind: 0,
        originExists: false,
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }
  }

  /**
   * Get working directory changes (not diff to base branch)
   */
  async getGitChanges(): Promise<GitChanges> {
    // If workspace doesn't exist or is not set up, return defaults
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      this.logger.warn('Workspace path does not exist when getting git changes', {
        workspacePath: this.workspacePath,
      })
      return {
        currentBranch: 'unknown',
        status: 'error',
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }

    try {
      // Import and use the working directory changes function
      const { getWorkingDirectoryChanges } = await import('../utils/git-utils.js')
      return getWorkingDirectoryChanges(this.workspacePath)
    } catch (_error) {
      // Fall back to defaults on error
      return {
        currentBranch: 'unknown',
        status: 'error',
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }
  }

  /**
   * Get the workspace path
   */
  getWorkspacePath(): string {
    return this.workspacePath
  }

  /**
   * Get the current commit hash
   */
  async getCurrentCommitHash(): Promise<string | null> {
    if (!this.workspacePath || !(await this.pathExists(this.workspacePath))) {
      this.logger.warn('Workspace path does not exist when getting commit hash', {
        workspacePath: this.workspacePath,
      })
      return null
    }

    try {
      const { getCommitHash } = await import('../utils/git-utils.js')
      return await getCommitHash(this.workspacePath, true) // Use short hash
    } catch (error) {
      this.logger.error('Failed to get commit hash', error as Error, {
        workspacePath: this.workspacePath,
      })
      return null
    }
  }

  /**
   * Update the workspace path (used during setup)
   */
  setWorkspacePath(newPath: string): void {
    if (this.workspacePath && this.workspacePath !== newPath) {
      this.logger.warn('Changing workspace path', {
        oldPath: this.workspacePath,
        newPath,
      })
    }
    ;(this as any).workspacePath = newPath
  }
}
