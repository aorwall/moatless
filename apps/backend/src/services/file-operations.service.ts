import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { GitChanges, GitStatus } from '@moatless/types'
import { WorkspaceError } from '@moatless/types'
import {
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

interface GitStatusCache {
  status: GitStatus
  timestamp: number
}

/**
 * Service responsible for all file system and git operations within a workspace.
 * This includes reading/writing files, listing directories, and git operations.
 */
export class FileOperations {
  private logger
  private gitStatusCache: GitStatusCache | null = null
  private readonly gitStatusCacheTTL = 60000 // 60 seconds

  constructor(
    private readonly taskId: string,
    private workspacePath: string,
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
    if (!this.workspacePath) {
      return false
    }

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
    if (!this.workspacePath) {
      return false
    }

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

      // Always return all fields, even if the file is not modified
      const result: {
        content: string
        oldContent?: string
        additions?: number
        deletions?: number
        isModified?: boolean
      } = { content }

      if (changeInfo.oldContent) {
        result.oldContent = changeInfo.oldContent
      }
      if (changeInfo.additions !== undefined) {
        result.additions = changeInfo.additions
      }
      if (changeInfo.deletions !== undefined) {
        result.deletions = changeInfo.deletions
      }
      if (changeInfo.isModified !== undefined) {
        result.isModified = changeInfo.isModified
      }

      return result
    } catch (error) {
      // If we can't get git info, return default values for change info
      this.logger.debug('Failed to get file change info', {
        filePath,
        error: error instanceof Error ? error.message : String(error),
      })

      // Return content with default values for change info (no optional fields means defaults)
      return {
        content,
        additions: 0,
        deletions: 0,
        isModified: false,
      }
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
      await fs.mkdir(path.dirname(fullPath), { recursive: true })
      await fs.writeFile(fullPath, content, 'utf-8')

      // Invalidate git status cache after file write
      this.invalidateGitStatusCache()
    } catch (error) {
      throw new Error(
        `Failed to write file ${filePath}: ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }

  /**
   * Search for files in the workspace
   */
  async searchFiles(request: FileSearchRequest) {
    if (!(this.workspacePath && (await this.pathExists(this.workspacePath)))) {
      throw new WorkspaceError(
        `Workspace path does not exist: ${this.workspacePath}`,
        this.taskId,
        400,
      )
    }

    const result = await searchFilesInDirectoryNew(this.workspacePath, request)

    // Transform to match FileSearchResponseSchema
    return {
      files: result.results.map((r) => ({
        path: r.path,
        matches: 1, // Each result counts as 1 match
      })),
      totalMatches: result.totalFound,
    }
  }

  /**
   * Get git diff for the workspace
   */
  async getGitDiff() {
    if (!(this.workspacePath && (await this.pathExists(this.workspacePath)))) {
      throw new WorkspaceError(
        `Workspace path does not exist: ${this.workspacePath}`,
        this.taskId,
        400,
      )
    }

    const result = await getGitDiffDetailed(this.workspacePath, this.originalBranch)

    // Transform to match GitDiffResponseSchema
    return {
      diff: result.diffString,
      files: result.files.map((f) => ({
        oldFile: f.oldFilename || f.filename,
        newFile: f.filename,
        hunks: [], // Simplified - could parse hunks from diffString if needed
      })),
    }
  }

  /**
   * Invalidate the git status cache
   */
  invalidateGitStatusCache(): void {
    this.gitStatusCache = null
    this.logger.debug('Git status cache invalidated', { taskId: this.taskId })
  }

  /**
   * Check if git status cache is valid
   */
  private isGitStatusCacheValid(): boolean {
    if (!this.gitStatusCache) {
      return false
    }
    const age = Date.now() - this.gitStatusCache.timestamp
    return age < this.gitStatusCacheTTL
  }

  /**
   * Check if the workspace has uncommitted git changes
   */
  async hasUncommittedChanges(): Promise<boolean> {
    // If workspace doesn't exist or is not set up, no changes
    if (!(this.workspacePath && (await this.pathExists(this.workspacePath)))) {
      return false
    }

    return hasUncommittedChanges(this.workspacePath)
  }

  /**
   * Get git status with additions, deletions, modifications
   */
  async getGitStatus(): Promise<GitStatus> {
    // Check cache first
    if (this.isGitStatusCacheValid() && this.gitStatusCache?.status) {
      this.logger.debug('Returning cached git status', { taskId: this.taskId })
      return this.gitStatusCache.status
    }

    // If workspace doesn't exist or is not set up, return defaults
    if (!(this.workspacePath && (await this.pathExists(this.workspacePath)))) {
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
      this.logger.debug('Fetching fresh git status', { taskId: this.taskId })

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

      const status: GitStatus = {
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

      // Cache the result
      this.gitStatusCache = {
        status,
        timestamp: Date.now(),
      }

      return status
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
    if (!(this.workspacePath && (await this.pathExists(this.workspacePath)))) {
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
    if (!(this.workspacePath && (await this.pathExists(this.workspacePath)))) {
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
    this.workspacePath = newPath
  }
}

// Factory function for creating file operations for a specific workspace
export function createFileOperations(
  taskId: string,
  workspacePath: string,
  originalBranch?: string,
): FileOperations {
  return new FileOperations(taskId, workspacePath, originalBranch)
}
