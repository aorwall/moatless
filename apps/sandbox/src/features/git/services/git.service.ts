import { access } from 'node:fs/promises'
import type { GitChanges, GitStatus } from '@moatless/types'
import type { SandboxConfig } from '../../../config/environment'
import { logger } from '../../../shared/logger'
import {
  checkForConflicts,
  getCommitHash,
  getGitDiffDetailed,
  getOriginSyncStatus,
  getWorkingDirectoryChanges,
} from '../utils/git.utils'

export interface GitDiffResult {
  files: Array<{
    oldFile: string
    newFile: string
    hunks: string[]
  }>
  diff?: string
}

export class GitService {
  constructor(private readonly config: SandboxConfig) {}

  private get workspacePath(): string {
    return this.config.workspaceDir
  }

  /**
   * Get git status for the workspace
   */
  async getStatus(): Promise<GitStatus> {
    // Check if workspace exists
    try {
      await access(this.workspacePath)
    } catch {
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
      const [syncStatus, hasConflicts, gitDiff] = await Promise.all([
        getOriginSyncStatus(this.workspacePath),
        checkForConflicts(this.workspacePath),
        getGitDiffDetailed(this.workspacePath),
      ])

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
    } catch {
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
   * Get git changes for the workspace
   */
  async getChanges(): Promise<GitChanges> {
    // Check if workspace exists
    try {
      await access(this.workspacePath)
    } catch {
      return {
        currentBranch: 'unknown',
        status: 'error',
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }

    try {
      return getWorkingDirectoryChanges(this.workspacePath)
    } catch {
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
   * Get git diff for the workspace
   */
  async getDiff(): Promise<GitDiffResult> {
    const result = await getGitDiffDetailed(this.workspacePath)

    const resultObj: GitDiffResult = {
      files: result.files.map((f) => ({
        oldFile: f.oldFilename || f.filename,
        newFile: f.filename,
        hunks: [] as string[], // Simplified - could parse hunks from diffString if needed
      })),
    }
    if (result.diffString) {
      resultObj.diff = result.diffString
    }
    return resultObj
  }

  /**
   * Get the current commit hash
   */
  async getCurrentCommitHash(): Promise<string | null> {
    // Check if workspace exists
    try {
      await access(this.workspacePath)
    } catch {
      return null
    }

    try {
      return await getCommitHash(this.workspacePath, true) // Use short hash
    } catch (error) {
      logger.debug(
        {
          taskId: this.config.taskId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to get commit hash',
      )
      return null
    }
  }
}
