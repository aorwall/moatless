import { exec } from 'node:child_process'
import { promisify } from 'node:util'
import { logger } from '../../../utils/logger'

const execAsync = promisify(exec)

/**
 * Check if a directory is a git repository
 */
export async function isGitRepository(path: string): Promise<boolean> {
  try {
    await execAsync('git rev-parse --git-dir', { cwd: path })
    return true
  } catch {
    return false
  }
}

/**
 * Check if a directory is a git worktree
 */
export async function isGitWorktree(path: string): Promise<boolean> {
  try {
    const { stdout } = await execAsync('git rev-parse --is-inside-work-tree', { cwd: path })
    return stdout.trim() === 'true'
  } catch {
    return false
  }
}

/**
 * Check if a branch exists in a repository
 */
export async function branchExists(repoPath: string, branchName: string): Promise<boolean> {
  try {
    await execAsync(`git show-ref --verify --quiet refs/heads/${branchName}`, { cwd: repoPath })
    return true
  } catch {
    return false
  }
}

/**
 * Create a new branch in a repository
 */
export async function createBranch(
  repoPath: string,
  branchName: string,
  baseBranch?: string,
): Promise<void> {
  const base = baseBranch || 'HEAD'
  logger.info('Creating branch', { repoPath, branchName, baseBranch: base })
  await execAsync(`git branch ${branchName} ${base}`, { cwd: repoPath })
}

/**
 * Create a git worktree
 */
export async function createGitWorktree(
  repoPath: string,
  worktreePath: string,
  branchName: string,
): Promise<void> {
  logger.info('Creating git worktree', { repoPath, worktreePath, branchName })
  await execAsync(`git worktree add ${worktreePath} ${branchName}`, { cwd: repoPath })
}

/**
 * Clone a git repository
 */
export async function cloneRepository(
  gitUrl: string,
  targetPath: string,
  options?: {
    branch?: string
    depth?: number
  },
): Promise<void> {
  logger.info('Cloning repository', { gitUrl, targetPath, options })

  let cmd = `git clone ${gitUrl} ${targetPath}`

  if (options?.branch) {
    cmd += ` --branch ${options.branch}`
  }

  if (options?.depth) {
    cmd += ` --depth ${options.depth}`
  }

  await execAsync(cmd)
}

/**
 * Get the current branch name
 */
export async function getCurrentBranch(repoPath: string): Promise<string> {
  const { stdout } = await execAsync('git rev-parse --abbrev-ref HEAD', { cwd: repoPath })
  return stdout.trim()
}

/**
 * Checkout a branch
 */
export async function checkoutBranch(repoPath: string, branchName: string): Promise<void> {
  logger.info('Checking out branch', { repoPath, branchName })
  await execAsync(`git checkout ${branchName}`, { cwd: repoPath })
}
