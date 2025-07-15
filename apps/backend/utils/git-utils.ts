import { exec } from 'node:child_process'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { promisify } from 'node:util'
import type { GitChanges, GitSyncStatus } from '@moatless/types'
import { logger } from './logger.js'

const execAsync = promisify(exec)

export interface GitDiffFile {
  status: 'added' | 'modified' | 'deleted' | 'renamed'
  filename: string
  oldFilename?: string // For renamed files
  additions: number
  deletions: number
  oldContent?: string
  newContent?: string
  isStaged: boolean
}

export interface GitDiffResponse {
  files: GitDiffFile[]
  totalChanges: number
  totalAdditions: number
  totalDeletions: number
  diffString: string
  compareBase: string
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

/**
 * Get untracked files and create diffs for them
 */
async function getUntrackedFilesDiff(repoPath: string): Promise<string> {
  try {
    const { stdout: untrackedOutput } = await execAsync(
      'git ls-files --others --exclude-standard',
      {
        cwd: repoPath,
        timeout: 10000,
      },
    )

    const untrackedFiles = untrackedOutput
      .trim()
      .split('\n')
      .filter((line) => line.trim())

    let untrackedDiffs = ''
    for (const filename of untrackedFiles) {
      if (filename) {
        try {
          // Read the content of untracked file
          const { stdout: fileContent } = await execAsync(`cat "${filename}"`, {
            cwd: repoPath,
            timeout: 5000,
            maxBuffer: 1024 * 1024, // 1MB buffer per file
          })

          // Create a diff-like output for untracked files
          untrackedDiffs += `diff --git a/${filename} b/${filename}\n`
          untrackedDiffs += 'new file mode 100644\n'
          untrackedDiffs += 'index 0000000..0000000\n'
          untrackedDiffs += '--- /dev/null\n'
          untrackedDiffs += `+++ b/${filename}\n`

          // Add file content with + prefix
          const lines = fileContent.split('\n')
          if (lines.length > 0) {
            untrackedDiffs += `@@ -0,0 +1,${lines.length} @@\n`
            for (const line of lines) {
              untrackedDiffs += `+${line}\n`
            }
          }
        } catch {
          // If we can't read the file, just add a header
          untrackedDiffs += `diff --git a/${filename} b/${filename}\n`
          untrackedDiffs += 'new file mode 100644\n'
          untrackedDiffs += 'Binary file\n'
        }
      }
    }

    return untrackedDiffs
  } catch {
    return ''
  }
}

/**
 * Get the best reference to compare against (origin/base > local base > merge base)
 * This ensures we always compare against the most up-to-date base when possible
 */
export async function getBestCompareReference(
  repoPath: string,
  baseBranch?: string,
): Promise<{ reference: string; type: 'origin' | 'local' | 'merge-base' }> {
  const base = baseBranch || (await detectBaseBranch(repoPath))
  const currentBranch = await getCurrentBranch(repoPath)
  const originBranch = `origin/${base}`

  // First, try to use origin/base if it exists
  try {
    await execAsync(`git rev-parse --verify ${originBranch}`, {
      cwd: repoPath,
      timeout: 5000,
    })
    return { reference: originBranch, type: 'origin' }
  } catch {
    // Origin branch doesn't exist, continue to local options
  }

  // For worktrees, try to get merge base for complete diff
  try {
    const mergeBase = await getMergeBase(repoPath, base, currentBranch)
    return { reference: mergeBase, type: 'merge-base' }
  } catch {
    // Fall back to local base branch
    return { reference: base, type: 'local' }
  }
}

/**
 * Check if origin remote exists and optionally fetch from it
 */
export async function checkAndFetchOrigin(
  repoPath: string,
  baseBranch?: string,
  shouldFetch = true,
): Promise<boolean> {
  try {
    await execAsync('git remote get-url origin', {
      cwd: repoPath,
      timeout: 5000,
    })

    if (shouldFetch) {
      const base = baseBranch || (await detectBaseBranch(repoPath))
      try {
        await execAsync(`git fetch origin ${base}`, {
          cwd: repoPath,
          timeout: 30000,
        })
      } catch (error) {
        logger.warn(`Failed to fetch origin/${base}`, {
          error: error instanceof Error ? error.message : String(error),
        })
      }
    }

    return true
  } catch {
    return false
  }
}

export async function getGitDiffString(
  repoPath: string,
  baseBranch?: string,
): Promise<[string, string]> {
  try {
    // Check if origin exists and fetch if available
    await checkAndFetchOrigin(repoPath, baseBranch, true)

    // Get the best reference to compare against (prioritizes origin/base)
    const { reference: compareBase, type } = await getBestCompareReference(repoPath, baseBranch)

    logger.debug('Using compare reference for git diff', { compareBase, type })

    // Using 'git diff compareBase' shows the complete difference between
    // the compare base and the current working directory state, which includes:
    // 1. All commits made on the current branch since it diverged
    // 2. All staged changes
    // 3. All unstaged changes
    const { stdout: diffOutput } = await execAsync(`git diff ${compareBase}`, {
      cwd: repoPath,
      timeout: 30000,
      maxBuffer: 10 * 1024 * 1024, // 10MB buffer
    })

    // Get untracked files and create diffs for them
    const untrackedDiffs = await getUntrackedFilesDiff(repoPath)

    // Combine regular diff with untracked files diff
    const combinedDiff = diffOutput + (untrackedDiffs ? `\n${untrackedDiffs}` : '')

    return [combinedDiff, compareBase]
  } catch (error) {
    // Try with master as fallback if no baseBranch was specified
    if (!baseBranch) {
      try {
        const { reference: fallbackCompare } = await getBestCompareReference(repoPath, 'master')
        const { stdout: diffOutput } = await execAsync(`git diff ${fallbackCompare}`, {
          cwd: repoPath,
          timeout: 30000,
          maxBuffer: 10 * 1024 * 1024,
        })

        const untrackedDiffs = await getUntrackedFilesDiff(repoPath)
        const combinedDiff = diffOutput + (untrackedDiffs ? `\n${untrackedDiffs}` : '')
        return [combinedDiff, fallbackCompare]
      } catch {
        // Final fallback: show only uncommitted changes
        const { stdout: diffOutput } = await execAsync('git diff', {
          cwd: repoPath,
          timeout: 30000,
          maxBuffer: 10 * 1024 * 1024,
        })

        const untrackedDiffs = await getUntrackedFilesDiff(repoPath)
        const combinedDiff = diffOutput + (untrackedDiffs ? `\n${untrackedDiffs}` : '')
        return [combinedDiff, 'HEAD']
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

export async function getCommitHash(repoPath: string, short = true): Promise<string> {
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

    const commitCount = Number.parseInt(stdout.trim(), 10)
    return commitCount > 0
  } catch (_error) {
    // Try with master if main fails
    if (!baseBranch) {
      try {
        const { stdout } = await execAsync('git rev-list --count master..HEAD', {
          cwd: repoPath,
          timeout: 5000,
        })

        const commitCount = Number.parseInt(stdout.trim(), 10)
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

    const commitCount = Number.parseInt(stdout.trim(), 10)
    return commitCount > 0
  } catch (_error) {
    // Try with master if main fails
    if (!baseBranch) {
      try {
        const { stdout } = await execAsync('git rev-list --count HEAD..master', {
          cwd: repoPath,
          timeout: 5000,
        })

        const commitCount = Number.parseInt(stdout.trim(), 10)
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
    } catch (error) {
      // If merge-tree fails, check if it's due to conflicts
      // Conflict information appears in both stdout and stderr depending on git version
      const err = error as { stderr?: string; stdout?: string }
      if (err.stderr?.includes('CONFLICT') || err.stdout?.includes('CONFLICT')) {
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
        } catch (error2) {
          const err2 = error2 as { stderr?: string; stdout?: string }
          if (err2.stderr?.includes('CONFLICT') || err2.stdout?.includes('CONFLICT')) {
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
      .map((n) => Number.parseInt(n, 10))
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
          .map((n) => Number.parseInt(n, 10))
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

export async function getMergeBase(
  repoPath: string,
  branch1: string,
  branch2: string,
): Promise<string> {
  try {
    const { stdout } = await execAsync(`git merge-base "${branch1}" "${branch2}"`, {
      cwd: repoPath,
      timeout: 5000,
    })
    return stdout.trim()
  } catch (error) {
    throw new Error(
      `Failed to get merge base: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function pruneWorktrees(repoPath: string): Promise<void> {
  try {
    // Prune worktree information for deleted worktrees
    await execAsync('git worktree prune', {
      cwd: repoPath,
      timeout: 5000,
    })
  } catch (error) {
    throw new Error(
      `Failed to prune worktrees: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export async function isGitWorktree(workspacePath: string): Promise<boolean> {
  try {
    // In a worktree, .git is a file (not a directory) containing the path to the real git directory
    const gitPath = path.join(workspacePath, '.git')
    const stat = await fs.stat(gitPath)

    if (stat.isFile()) {
      // Read the .git file to confirm it's a worktree
      const content = await fs.readFile(gitPath, 'utf-8')
      return content.startsWith('gitdir:')
    }

    return false
  } catch {
    // If any error occurs (file doesn't exist, etc.), it's not a worktree
    return false
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

export async function isBranchCheckedOut(repoPath: string, branchName: string): Promise<boolean> {
  try {
    const { stdout } = await execAsync('git worktree list --porcelain', {
      cwd: repoPath,
      timeout: 10000,
    })

    const lines = stdout.trim().split('\n')
    let currentBranch = ''

    for (const line of lines) {
      if (line.startsWith('branch ')) {
        currentBranch = line.substring(7).trim()
        if (currentBranch === `refs/heads/${branchName}`) {
          return true
        }
      }
    }

    return false
  } catch (error) {
    logger.debug('Failed to check if branch is checked out', { error, branchName })
    return false
  }
}

export async function createGitWorktree(
  repoPath: string,
  workspacePath: string,
  branchName?: string,
  newBranchName?: string,
): Promise<void> {
  try {
    if (branchName) {
      // Check if the branch is already checked out
      const isCheckedOut = await isBranchCheckedOut(repoPath, branchName)

      if (isCheckedOut) {
        // Branch is already checked out, create a new branch based on it
        if (!newBranchName) {
          throw new Error(
            `Branch '${branchName}' is already checked out and no new branch name provided`,
          )
        }
        logger.debug('Branch already checked out, creating new branch from it', {
          originalBranch: branchName,
          newBranch: newBranchName,
        })
        await execAsync(
          `git worktree add -b "${newBranchName}" "${workspacePath}" "${branchName}"`,
          {
            cwd: repoPath,
            timeout: 30000,
          },
        )
      } else {
        // Create worktree from existing branch (no -b flag)
        logger.debug('Using existing branch for worktree', { branch: branchName })
        await execAsync(`git worktree add "${workspacePath}" "${branchName}"`, {
          cwd: repoPath,
          timeout: 30000,
        })
      }
    } else if (newBranchName) {
      // Create new branch for worktree
      logger.debug('Creating new branch for worktree', { branch: newBranchName })
      await execAsync(`git worktree add -b "${newBranchName}" "${workspacePath}"`, {
        cwd: repoPath,
        timeout: 30000,
      })
    } else {
      throw new Error('Either branchName or newBranchName must be provided')
    }

    logger.info('Git worktree created successfully', {
      workspacePath,
      branchName,
      newBranchName,
    })
  } catch (error) {
    logger.error('Failed to create git worktree', error as Error, {
      repoPath,
      workspacePath,
      branchName,
      newBranchName,
    })
    throw new Error(
      `Failed to create worktree: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export interface GitSyncStatusInfo {
  status: GitSyncStatus
  commitsAhead: number
  commitsBehind: number
  hasUncommittedChanges: boolean
  currentBranch: string
  baseBranch: string
  originExists: boolean
}

/**
 * Check sync status with origin/main or origin/master
 */
export async function getOriginSyncStatus(
  repoPath: string,
  baseBranch?: string,
): Promise<GitSyncStatusInfo> {
  try {
    // Get current branch
    const currentBranch = await getCurrentBranch(repoPath)

    // Detect base branch if not provided
    const base = baseBranch || (await detectBaseBranch(repoPath))

    // Check if origin exists and fetch if available
    const originExists = await checkAndFetchOrigin(repoPath, base, true)

    if (!originExists) {
      const hasChanges = await hasUncommittedChanges(repoPath)
      return {
        status: hasChanges ? 'uncommitted_changes' : 'synced',
        commitsAhead: 0,
        commitsBehind: 0,
        hasUncommittedChanges: hasChanges,
        currentBranch,
        baseBranch: base,
        originExists: false,
      }
    }

    // Check for uncommitted changes first
    const hasChanges = await hasUncommittedChanges(repoPath)

    // Get commit counts relative to origin/base
    const { ahead, behind } = await getOriginCommitCounts(repoPath, base)

    // Determine sync status
    let status: GitSyncStatus
    if (hasChanges) {
      status = 'uncommitted_changes'
    } else if (ahead > 0 && behind > 0) {
      status = 'diverged'
    } else if (ahead > 0) {
      status = 'ahead'
    } else if (behind > 0) {
      status = 'behind'
    } else {
      status = 'synced'
    }

    return {
      status,
      commitsAhead: ahead,
      commitsBehind: behind,
      hasUncommittedChanges: hasChanges,
      currentBranch,
      baseBranch: base,
      originExists: true,
    }
  } catch (error) {
    logger.error('Failed to get origin sync status', error as Error)
    const hasChanges = await hasUncommittedChanges(repoPath).catch(() => false)
    const currentBranch = await getCurrentBranch(repoPath).catch(() => 'unknown')
    const base = baseBranch || 'main'

    return {
      status: 'error',
      commitsAhead: 0,
      commitsBehind: 0,
      hasUncommittedChanges: hasChanges,
      currentBranch,
      baseBranch: base,
      originExists: false,
    }
  }
}

/**
 * Get commit counts ahead/behind relative to origin/base branch
 */
export async function getOriginCommitCounts(
  repoPath: string,
  baseBranch?: string,
): Promise<{ ahead: number; behind: number }> {
  try {
    const base = baseBranch || (await detectBaseBranch(repoPath))
    const originBranch = `origin/${base}`

    // Check if origin branch exists
    try {
      await execAsync(`git rev-parse --verify ${originBranch}`, {
        cwd: repoPath,
        timeout: 5000,
      })
    } catch {
      // Origin branch doesn't exist, fall back to local branch comparison
      return getCommitCounts(repoPath, base)
    }

    // Get commits ahead and behind relative to origin
    const { stdout } = await execAsync(`git rev-list --left-right --count ${originBranch}...HEAD`, {
      cwd: repoPath,
      timeout: 5000,
    })

    const [behind, ahead] = stdout
      .trim()
      .split('\t')
      .map((n) => Number.parseInt(n, 10))

    return { ahead, behind }
  } catch (error) {
    logger.warn('Failed to get origin commit counts, falling back to local', {
      error: error instanceof Error ? error.message : String(error),
    })
    // Fall back to local branch comparison
    return getCommitCounts(repoPath, baseBranch)
  }
}

/**
 * Get detailed git diff with file contents and staging information
 */
export async function getGitDiffDetailed(
  repoPath: string,
  baseBranch?: string,
): Promise<GitDiffResponse> {
  try {
    // Get the basic diff string and compare base
    const [diffString, compareBase] = await getGitDiffString(repoPath, baseBranch)

    // Get staging status for files
    const { staged, untracked } = await getGitStatus(repoPath)
    const stagedSet = new Set(staged)

    // Parse the diff to extract file information
    const files: GitDiffFile[] = []
    let totalAdditions = 0
    let totalDeletions = 0

    // Split diff into file chunks
    const diffChunks = diffString.split(/\ndiff --git /).filter((chunk) => chunk.trim())

    for (const chunk of diffChunks) {
      const lines = chunk.split('\n')
      if (lines.length === 0) continue

      // Parse file names from the diff header
      const fileMatch = lines[0].match(/a\/(.+) b\/(.+)/)
      if (!fileMatch) continue

      const oldFileName = fileMatch[1]
      const newFileName = fileMatch[2]
      const filename = newFileName

      // Determine file status
      let status: GitDiffFile['status'] = 'modified'
      let isNewFile = false
      let isDeletedFile = false
      let isRenamedFile = false

      // Check for file status indicators in the diff
      for (const line of lines.slice(1, 5)) {
        if (line.includes('new file mode')) {
          status = 'added'
          isNewFile = true
          break
        }
        if (line.includes('deleted file mode')) {
          status = 'deleted'
          isDeletedFile = true
          break
        }
        if (line.includes('rename from') || line.includes('rename to')) {
          status = 'renamed'
          isRenamedFile = true
          break
        }
      }

      // Count additions and deletions
      let additions = 0
      let deletions = 0
      const addedLines: string[] = []
      const removedLines: string[] = []

      for (const line of lines) {
        if (line.startsWith('+') && !line.startsWith('+++')) {
          additions++
          addedLines.push(line.substring(1))
        } else if (line.startsWith('-') && !line.startsWith('---')) {
          deletions++
          removedLines.push(line.substring(1))
        }
      }

      totalAdditions += additions
      totalDeletions += deletions

      // Get file contents for new and deleted files
      let oldContent: string | undefined
      let newContent: string | undefined

      if (isNewFile || isDeletedFile) {
        try {
          if (isNewFile) {
            // For new files, try to read current content
            const filePath = path.join(repoPath, filename)
            try {
              newContent = await fs.readFile(filePath, 'utf-8')
            } catch {
              // If can't read, use the added lines from diff
              newContent = addedLines.join('\n')
            }
          } else if (isDeletedFile) {
            // For deleted files, get content from git
            try {
              const { stdout } = await execAsync(`git show ${compareBase}:"${filename}"`, {
                cwd: repoPath,
                timeout: 5000,
                maxBuffer: 1024 * 1024, // 1MB
              })
              oldContent = stdout
            } catch {
              // If can't get from git, use the removed lines from diff
              oldContent = removedLines.join('\n')
            }
          }
        } catch (error) {
          logger.debug('Error getting file content for diff', {
            filename,
            error: error instanceof Error ? error.message : String(error),
          })
        }
      }

      // Check if file is staged
      const isStaged = stagedSet.has(filename)

      const diffFile: GitDiffFile = {
        status,
        filename,
        oldFilename: isRenamedFile && oldFileName !== newFileName ? oldFileName : undefined,
        additions,
        deletions,
        oldContent,
        newContent,
        isStaged,
      }

      files.push(diffFile)
    }

    // Handle untracked files that weren't in the diff
    for (const untrackedFile of untracked) {
      // Check if we already have this file in our list
      if (!files.some((f) => f.filename === untrackedFile)) {
        try {
          const filePath = path.join(repoPath, untrackedFile)
          const content = await fs.readFile(filePath, 'utf-8')
          // Count lines the same way git does - split by newline and count non-empty result
          // If file ends with newline, split gives an extra empty string at the end
          const splitLines = content.split('\n')
          const lines = content.endsWith('\n') ? splitLines.length - 1 : splitLines.length

          files.push({
            status: 'added',
            filename: untrackedFile,
            additions: lines,
            deletions: 0,
            newContent: content,
            isStaged: false,
          })

          totalAdditions += lines
        } catch (error) {
          logger.debug('Error reading untracked file', {
            filename: untrackedFile,
            error: error instanceof Error ? error.message : String(error),
          })
        }
      }
    }

    return {
      files,
      totalChanges: files.length,
      totalAdditions,
      totalDeletions,
      diffString,
      compareBase,
    }
  } catch (error) {
    throw new Error(
      `Failed to get detailed git diff: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

export interface GitSyncResult {
  success: boolean
  conflicts: boolean
  message: string
  commitsBehind?: number
  commitsAhead?: number
}

/**
 * Safely sync workspace with main branch if no conflicts exist
 * Checks for conflicts, uncommitted changes, and only syncs when safe
 */
export async function syncWithMain(repoPath: string, baseBranch?: string): Promise<GitSyncResult> {
  try {
    // Check current sync status
    const syncStatus = await getOriginSyncStatus(repoPath, baseBranch)

    // If already synced, nothing to do
    if (syncStatus.status === 'synced') {
      return {
        success: true,
        conflicts: false,
        message: 'Already up to date with main',
        commitsBehind: syncStatus.commitsBehind,
        commitsAhead: syncStatus.commitsAhead,
      }
    }

    // Check for uncommitted changes first
    if (syncStatus.hasUncommittedChanges) {
      return {
        success: false,
        conflicts: false,
        message: 'Cannot sync: uncommitted changes detected. Please commit or stash changes first.',
        commitsBehind: syncStatus.commitsBehind,
        commitsAhead: syncStatus.commitsAhead,
      }
    }

    // Check for conflicts before attempting sync
    const hasConflicts = await checkForConflicts(repoPath, baseBranch)
    if (hasConflicts) {
      return {
        success: false,
        conflicts: true,
        message: 'Cannot sync: merge conflicts detected. Manual resolution required.',
        commitsBehind: syncStatus.commitsBehind,
        commitsAhead: syncStatus.commitsAhead,
      }
    }

    // Only sync if we're behind or diverged (and no conflicts)
    if (syncStatus.status === 'behind' || syncStatus.status === 'diverged') {
      const base = baseBranch || syncStatus.baseBranch
      const originBranch = `origin/${base}`

      // Ensure we have the latest changes
      await checkAndFetchOrigin(repoPath, base, true)

      // Attempt to merge origin/main into current branch
      await execAsync(`git merge ${originBranch}`, {
        cwd: repoPath,
        timeout: 30000,
      })

      logger.info('Successfully synced workspace with main', {
        repoPath,
        originBranch,
        commitsBehind: syncStatus.commitsBehind,
      })

      return {
        success: true,
        conflicts: false,
        message: `Successfully synced with ${originBranch}. Updated ${syncStatus.commitsBehind} commits.`,
        commitsBehind: syncStatus.commitsBehind,
        commitsAhead: syncStatus.commitsAhead,
      }
    }

    // For 'ahead' status, no sync needed
    return {
      success: true,
      conflicts: false,
      message: `No sync needed. Status: ${syncStatus.status}`,
      commitsBehind: syncStatus.commitsBehind,
      commitsAhead: syncStatus.commitsAhead,
    }
  } catch (error) {
    logger.error('Failed to sync with main', error as Error, { repoPath, baseBranch })

    // Try to abort merge if it failed
    try {
      await execAsync('git merge --abort', {
        cwd: repoPath,
        timeout: 5000,
      })
    } catch {
      // Ignore abort errors
    }

    return {
      success: false,
      conflicts: false,
      message: `Sync failed: ${error instanceof Error ? error.message : String(error)}`,
    }
  }
}

/**
 * Get working directory changes (staged + unstaged + untracked)
 * This shows actual changes in the working directory, not a diff to the base branch
 */
export async function getWorkingDirectoryChanges(repoPath: string): Promise<GitChanges> {
  try {
    // Get current branch
    const currentBranch = await getCurrentBranch(repoPath)

    // Get git status to check if working directory is clean
    const gitStatus = await getGitStatus(repoPath)

    // If working directory is clean, return early
    if (gitStatus.clean) {
      return {
        currentBranch,
        status: 'clean',
        additions: 0,
        deletions: 0,
        modifications: 0,
      }
    }

    // Use git diff --stat to get statistics for tracked files
    // This includes both staged and unstaged changes
    const { stdout: diffStatOutput } = await execAsync('git diff --stat HEAD --numstat', {
      cwd: repoPath,
      timeout: 10000,
    })

    let totalAdditions = 0
    let totalDeletions = 0
    const modifiedFiles = new Set<string>()

    // Parse numstat output (format: additions<TAB>deletions<TAB>filename)
    const lines = diffStatOutput
      .trim()
      .split('\n')
      .filter((line) => line.trim())

    for (const line of lines) {
      const parts = line.split('\t')
      if (parts.length >= 3) {
        const additions = Number.parseInt(parts[0], 10) || 0
        const deletions = Number.parseInt(parts[1], 10) || 0
        const filename = parts[2]

        if (!Number.isNaN(additions) && !Number.isNaN(deletions)) {
          totalAdditions += additions
          totalDeletions += deletions
          modifiedFiles.add(filename)
        }
      }
    }

    // Count untracked files and their lines
    for (const untrackedFile of gitStatus.untracked) {
      try {
        const filePath = path.join(repoPath, untrackedFile)
        const content = await fs.readFile(filePath, 'utf-8')
        const lines = content.split('\n').length
        totalAdditions += lines
        modifiedFiles.add(untrackedFile)
      } catch (error) {
        logger.debug('Error reading untracked file for line count', {
          filename: untrackedFile,
          error: error instanceof Error ? error.message : String(error),
        })
      }
    }

    return {
      currentBranch,
      status: 'modified',
      additions: totalAdditions,
      deletions: totalDeletions,
      modifications: modifiedFiles.size,
    }
  } catch (error) {
    logger.error('Failed to get working directory changes', error as Error, { repoPath })

    // Return safe defaults if git operations fail
    const currentBranch = await getCurrentBranch(repoPath).catch(() => 'unknown')
    return {
      currentBranch,
      status: 'error',
      additions: 0,
      deletions: 0,
      modifications: 0,
    }
  }
}
