import { exec } from 'node:child_process'
import { promisify } from 'node:util'
import type { GitStatus } from '@shared/types/api.js'

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

export async function getGitDiffFiles(
  repoPath: string,
  baseBranch?: string,
): Promise<GitDiffResponse> {
  try {
    // Get the base branch (default to main)
    const base = baseBranch || 'main'

    // Get file status changes against base branch
    const { stdout: statusOutput } = await execAsync(`git diff --name-status ${base}`, {
      cwd: repoPath,
      timeout: 10000,
    })

    // Get detailed diff stats against base branch
    const { stdout: statOutput } = await execAsync(`git diff --numstat ${base}`, {
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

    // For worktrees, we need to show ALL changes made on this branch since creation.
    // Using 'git diff base' (without dots) shows the complete difference between
    // the base branch and the current working directory state, which includes:
    // 1. All commits made on the current branch
    // 2. All staged changes
    // 3. All unstaged changes
    // This is exactly what we want for reviewing all work done in a worktree.
    const { stdout } = await execAsync(`git diff ${base}`, {
      cwd: repoPath,
      timeout: 30000,
      maxBuffer: 10 * 1024 * 1024, // 10MB buffer
    })

    return [stdout, base]
  } catch (error) {
    // Try with master if main fails
    if (!baseBranch) {
      try {
        const { stdout } = await execAsync('git diff master', {
          cwd: repoPath,
          timeout: 30000,
          maxBuffer: 10 * 1024 * 1024,
        })
        return [stdout, 'master']
      } catch {
        // Fall back to showing uncommitted changes only
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

    const lines = stdout.split('\n').filter((line) => line.length > 0)
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

export async function hasUnmergedCommits(repoPath: string, baseBranch?: string): Promise<boolean> {
  try {
    // Get the base branch (default to main)
    const base = baseBranch || 'main'

    // Check if there are commits in current branch that are not in base branch
    // Using git rev-list to count commits
    const { stdout } = await execAsync(`git rev-list --count ${base}..HEAD`, {
      cwd: repoPath,
      timeout: 5000,
    })

    const commitCount = parseInt(stdout.trim(), 10)
    return commitCount > 0
  } catch (error) {
    // Try with master if main fails
    if (!baseBranch) {
      try {
        const { stdout } = await execAsync('git rev-list --count master..HEAD', {
          cwd: repoPath,
          timeout: 5000,
        })

        const commitCount = parseInt(stdout.trim(), 10)
        return commitCount > 0
      } catch {
        // If both main and master fail, assume no unmerged commits
        return false
      }
    }
    return false
  }
}

export async function isBehindBase(repoPath: string, baseBranch?: string): Promise<boolean> {
  try {
    // Get the base branch (default to main)
    const base = baseBranch || 'main'

    // Check if there are commits in base branch that are not in current branch
    // Using git rev-list to count commits
    const { stdout } = await execAsync(`git rev-list --count HEAD..${base}`, {
      cwd: repoPath,
      timeout: 5000,
    })

    const commitCount = parseInt(stdout.trim(), 10)
    return commitCount > 0
  } catch (error) {
    // Try with master if main fails
    if (!baseBranch) {
      try {
        const { stdout } = await execAsync('git rev-list --count HEAD..master', {
          cwd: repoPath,
          timeout: 5000,
        })

        const commitCount = parseInt(stdout.trim(), 10)
        return commitCount > 0
      } catch {
        // If both main and master fail, assume not behind
        return false
      }
    }
    return false
  }
}

export async function checkForConflicts(repoPath: string, baseBranch?: string): Promise<boolean> {
  try {
    const base = baseBranch || 'main'

    // Use git merge-tree to check for conflicts without modifying working tree
    // We check if merging current branch (HEAD) into base would cause conflicts
    try {
      const { stdout, stderr } = await execAsync(`git merge-tree --write-tree ${base} HEAD`, {
        cwd: repoPath,
        timeout: 10000,
      })

      // git merge-tree with --write-tree succeeds even with conflicts
      // Conflicts are indicated by "CONFLICT" in stderr output
      return stderr.includes('CONFLICT') || stdout.includes('CONFLICT')
    } catch (error: any) {
      // If merge-tree fails, check if it's due to conflicts
      // Conflict information appears in both stdout and stderr depending on git version
      if (error.stderr?.includes('CONFLICT') || error.stdout?.includes('CONFLICT')) {
        return true
      }

      // Try with master if main fails
      if (!baseBranch) {
        try {
          const { stdout, stderr } = await execAsync('git merge-tree --write-tree master HEAD', {
            cwd: repoPath,
            timeout: 10000,
          })

          return stderr.includes('CONFLICT') || stdout.includes('CONFLICT')
        } catch (error2: any) {
          if (error2.stderr?.includes('CONFLICT') || error2.stdout?.includes('CONFLICT')) {
            return true
          }
        }
      }

      return false
    }
  } catch {
    return false
  }
}

export async function getCommitCounts(
  repoPath: string,
  baseBranch?: string,
): Promise<{ ahead: number; behind: number }> {
  try {
    const base = baseBranch || 'main'

    // Get commits ahead and behind in one command
    const { stdout } = await execAsync(`git rev-list --left-right --count ${base}...HEAD`, {
      cwd: repoPath,
      timeout: 5000,
    })

    const [behind, ahead] = stdout
      .trim()
      .split('\t')
      .map((n) => parseInt(n, 10))
    return { ahead, behind }
  } catch {
    // Try with master if main fails
    if (!baseBranch) {
      try {
        const { stdout } = await execAsync('git rev-list --left-right --count master...HEAD', {
          cwd: repoPath,
          timeout: 5000,
        })

        const [behind, ahead] = stdout
          .trim()
          .split('\t')
          .map((n) => parseInt(n, 10))
        return { ahead, behind }
      } catch {
        return { ahead: 0, behind: 0 }
      }
    }
    return { ahead: 0, behind: 0 }
  }
}

export async function detectBaseBranch(repoPath: string): Promise<string> {
  // Check if main branch exists
  try {
    await execAsync('git rev-parse --verify main', {
      cwd: repoPath,
      timeout: 5000,
    })
    return 'main'
  } catch {
    // Check if master branch exists
    try {
      await execAsync('git rev-parse --verify master', {
        cwd: repoPath,
        timeout: 5000,
      })
      return 'master'
    } catch {
      // Default to main if neither exists
      return 'main'
    }
  }
}

export async function getComprehensiveGitStatus(
  repoPath: string,
  providedBaseBranch?: string,
): Promise<GitStatus> {
  try {
    // Get current branch
    const currentBranch = await getCurrentBranch(repoPath)

    // Detect base branch if not provided
    const baseBranch = providedBaseBranch || (await detectBaseBranch(repoPath))

    // Get all the git information in parallel for efficiency
    const [uncommittedChanges, commitCounts, conflicts] = await Promise.all([
      hasUncommittedChanges(repoPath),
      getCommitCounts(repoPath, baseBranch),
      checkForConflicts(repoPath, baseBranch),
    ])

    return {
      currentBranch,
      baseBranch,
      hasUncommittedChanges: uncommittedChanges,
      hasUnmergedCommits: commitCounts.ahead > 0,
      isBehindBase: commitCounts.behind > 0,
      hasConflicts: conflicts,
      commitsAhead: commitCounts.ahead,
      commitsBehind: commitCounts.behind,
    }
  } catch (error) {
    // Return safe defaults if git operations fail
    return {
      currentBranch: 'unknown',
      baseBranch: 'main',
      hasUncommittedChanges: false,
      hasUnmergedCommits: false,
      isBehindBase: false,
      hasConflicts: false,
      commitsAhead: 0,
      commitsBehind: 0,
    }
  }
}

export async function mergeGitBranch(
  repoPath: string,
  sourceBranch: string,
  targetBranch: string,
  commitMessage?: string,
): Promise<string> {
  try {
    // First, check if we're in a worktree
    const gitPath = `${repoPath}/.git`
    let mainRepoPath = repoPath

    try {
      const { stdout: gitFileContent } = await execAsync(`cat "${gitPath}"`, {
        cwd: repoPath,
        timeout: 5000,
      })

      // If .git is a file (worktree), extract main repo path
      if (gitFileContent.startsWith('gitdir:')) {
        const gitDir = gitFileContent.split(':')[1].trim()
        // Extract main repo path from worktree git dir
        // Format: /path/to/main/.git/worktrees/name -> /path/to/main
        const pathParts = gitDir.split('/')
        const worktreesIndex = pathParts.lastIndexOf('worktrees')
        if (worktreesIndex > 0) {
          mainRepoPath = pathParts.slice(0, worktreesIndex - 1).join('/')
        }
      }
    } catch {
      // .git is a directory, we're in the main repo
    }

    // Checkout target branch in the main repository
    await execAsync(`git checkout "${targetBranch}"`, {
      cwd: mainRepoPath,
      timeout: 10000,
    })

    // Prepare merge command
    const mergeMessage = commitMessage || `Merge branch '${sourceBranch}' into ${targetBranch}`
    await execAsync(`git merge --no-ff "${sourceBranch}" -m "${mergeMessage}"`, {
      cwd: mainRepoPath,
      timeout: 30000,
    })

    // Get the merge commit hash
    const { stdout: commitHash } = await execAsync('git rev-parse HEAD', {
      cwd: mainRepoPath,
      timeout: 5000,
    })

    return commitHash.trim()
  } catch (error) {
    // Try to abort merge if it failed
    try {
      await execAsync('git merge --abort', {
        cwd: repoPath,
        timeout: 5000,
      })
    } catch {
      // Ignore abort errors
    }

    throw new Error(
      `Failed to merge branch: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}
