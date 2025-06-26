import { exec } from 'child_process'
import { promisify } from 'util'

const execAsync = promisify(exec)

export interface GitDiffFile {
  status: string
  filename: string
  additions?: number
  deletions?: number
}

export interface GitDiffResponse {
  files: GitDiffFile[]
  totalChanges: number
  totalAdditions: number
  totalDeletions: number
}

export async function getGitDiffFiles(repoPath: string): Promise<GitDiffResponse> {
  try {
    // Get file status changes
    const { stdout: statusOutput } = await execAsync('git diff --name-status', {
      cwd: repoPath,
      timeout: 10000,
    })

    // Get detailed diff stats
    const { stdout: statOutput } = await execAsync('git diff --numstat', {
      cwd: repoPath,
      timeout: 10000,
    })

    const statusLines = statusOutput
      .trim()
      .split('\n')
      .filter((line) => line.trim())
    const statLines = statOutput
      .trim()
      .split('\n')
      .filter((line) => line.trim())

    const files: GitDiffFile[] = []
    let totalAdditions = 0
    let totalDeletions = 0

    // Parse status changes
    const fileStats = new Map<string, { additions: number; deletions: number }>()
    for (const statLine of statLines) {
      const [additions, deletions, filename] = statLine.split('\t')
      if (filename) {
        const add = additions === '-' ? 0 : parseInt(additions, 10)
        const del = deletions === '-' ? 0 : parseInt(deletions, 10)
        fileStats.set(filename, { additions: add, deletions: del })
        totalAdditions += add
        totalDeletions += del
      }
    }

    // Combine status and stats
    for (const statusLine of statusLines) {
      const [status, filename] = statusLine.split('\t')
      if (filename) {
        const stats = fileStats.get(filename) || { additions: 0, deletions: 0 }
        files.push({
          status,
          filename,
          additions: stats.additions,
          deletions: stats.deletions,
        })
      }
    }

    return {
      files,
      totalChanges: files.length,
      totalAdditions,
      totalDeletions,
    }
  } catch (error) {
    throw new Error(`Git diff failed: ${error instanceof Error ? error.message : String(error)}`)
  }
}

export async function getCurrentBranch(repoPath: string): Promise<string> {
  try {
    const { stdout } = await execAsync('git branch --show-current', {
      cwd: repoPath,
      timeout: 5000,
    })
    return stdout.trim()
  } catch (error) {
    throw new Error(
      `Failed to get current branch: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function hasUncommittedChanges(repoPath: string): Promise<boolean> {
  try {
    const { stdout } = await execAsync('git status --porcelain', {
      cwd: repoPath,
      timeout: 5000,
    })
    return stdout.trim().length > 0
  } catch (error) {
    throw new Error(
      `Failed to check uncommitted changes: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function getGitDiffString(
  repoPath: string,
  baseBranch?: string,
): Promise<[string, string]> {
  try {
    // Get the current branch if base branch not specified
    const base = baseBranch || 'main'

    // Get the diff
    const { stdout } = await execAsync(`git diff ${base}...HEAD`, {
      cwd: repoPath,
      timeout: 30000,
      maxBuffer: 10 * 1024 * 1024, // 10MB buffer
    })

    return [stdout, base]
  } catch (error) {
    // Try with master if main fails
    if (!baseBranch) {
      try {
        const { stdout } = await execAsync('git diff master...HEAD', {
          cwd: repoPath,
          timeout: 30000,
          maxBuffer: 10 * 1024 * 1024,
        })
        return [stdout, 'master']
      } catch {
        // Fall back to just git diff
        const { stdout } = await execAsync('git diff', {
          cwd: repoPath,
          timeout: 30000,
          maxBuffer: 10 * 1024 * 1024,
        })
        return [stdout, 'HEAD']
      }
    }
    throw new Error(
      `Failed to get git diff: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function isGitRepository(repoPath: string): Promise<boolean> {
  try {
    await execAsync('git rev-parse --git-dir', {
      cwd: repoPath,
      timeout: 5000,
    })
    return true
  } catch {
    return false
  }
}

export async function createBranch(repoPath: string, branchName: string): Promise<void> {
  try {
    await execAsync(`git checkout -b "${branchName}"`, {
      cwd: repoPath,
      timeout: 10000,
    })
  } catch (error) {
    throw new Error(
      `Failed to create branch: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function switchBranch(repoPath: string, branchName: string): Promise<void> {
  try {
    await execAsync(`git checkout "${branchName}"`, {
      cwd: repoPath,
      timeout: 10000,
    })
  } catch (error) {
    throw new Error(
      `Failed to switch branch: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function getCommitHash(repoPath: string, short: boolean = true): Promise<string> {
  try {
    const command = short ? 'git rev-parse --short HEAD' : 'git rev-parse HEAD'
    const { stdout } = await execAsync(command, {
      cwd: repoPath,
      timeout: 5000,
    })
    return stdout.trim()
  } catch (error) {
    throw new Error(
      `Failed to get commit hash: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function getBranchList(repoPath: string): Promise<string[]> {
  try {
    const { stdout } = await execAsync('git branch --list', {
      cwd: repoPath,
      timeout: 5000,
    })

    return stdout
      .split('\n')
      .map((line) => line.replace(/^\*?\s+/, '').trim())
      .filter((branch) => branch && !branch.startsWith('('))
  } catch (error) {
    throw new Error(
      `Failed to get branch list: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function getGitStatus(repoPath: string): Promise<{
  clean: boolean
  modified: string[]
  untracked: string[]
  staged: string[]
}> {
  try {
    const { stdout } = await execAsync('git status --porcelain', {
      cwd: repoPath,
      timeout: 5000,
    })

    const lines = stdout
      .split('\n')
      .filter((line) => line.length > 0)
    const modified: string[] = []
    const untracked: string[] = []
    const staged: string[] = []

    for (const line of lines) {
      const status = line.substring(0, 2)
      const filename = line.substring(2).trim()
      if (status[0] !== ' ' && status[0] !== '?') {
        staged.push(filename)
      }
      if (status[1] === 'M' || status[1] === 'D') {
        modified.push(filename)
      }
      if (status === '??') {
        untracked.push(filename)
      }
    }

    return {
      clean: lines.length === 0,
      modified,
      untracked,
      staged,
    }
  } catch (error) {
    throw new Error(
      `Failed to get git status: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}
