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
  logger.info({ repoPath, branchName, baseBranch: base }, 'Creating branch')
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
  logger.info({ repoPath, worktreePath, branchName }, 'Creating git worktree')
  await execAsync(`git worktree add ${worktreePath} ${branchName}`, { cwd: repoPath })
}

/**
 * Clone a git repository
 *
 * If GITHUB_TOKEN env var is set for GitHub URLs, it will be used for authentication.
 */
export async function cloneRepository(
  gitUrl: string,
  targetPath: string,
  options?: {
    branch?: string
    depth?: number
  },
): Promise<void> {
  // Build authenticated URL if GITHUB_TOKEN is available for GitHub URLs
  let cloneUrl = gitUrl
  const githubToken = process.env['GITHUB_TOKEN']

  if (githubToken && gitUrl.includes('github.com')) {
    // Insert token into GitHub URL for authentication
    cloneUrl = gitUrl.replace('https://github.com/', `https://${githubToken}@github.com/`)
    logger.info({ targetPath, options }, 'Cloning repository with GitHub token authentication')
  } else {
    logger.info({ gitUrl, targetPath, options }, 'Cloning repository')
  }

  let cmd = `git clone ${cloneUrl} ${targetPath}`

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
  logger.info({ repoPath, branchName }, 'Checking out branch')
  await execAsync(`git checkout ${branchName}`, { cwd: repoPath })
}
