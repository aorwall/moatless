import { exec } from 'node:child_process'
import { access, mkdir } from 'node:fs/promises'
import { promisify } from 'node:util'
import type { SandboxConfig } from '../../../config/environment'
import { logger } from '../../../shared/logger'

const execAsync = promisify(exec)

export type ProvisioningStatus = 'pending' | 'ready' | 'failed'

export interface ProvisioningResult {
  status: ProvisioningStatus
  workspaceDir: string
  error?: string
}

/**
 * Provisions the workspace at sandbox startup.
 * - Creates workspace directory if needed
 * - Clones git repository if configured
 * - Checks out specified branch
 * - Runs setup commands from REPO_CONFIG (if provided)
 */
export async function provisionWorkspace(config: SandboxConfig): Promise<ProvisioningResult> {
  const { workspaceDir, gitCloneUrl, gitBranch, repoConfig } = config

  logger.info(
    {
      workspaceDir,
      gitCloneUrl,
      gitBranch,
      hasRepoConfig: !!repoConfig,
      setupCommands: repoConfig?.codeExecution?.setupCommands,
    },
    'Starting workspace provisioning',
  )

  try {
    // Ensure workspace directory exists
    await mkdir(workspaceDir, { recursive: true })

    // Clone repository if configured
    if (gitCloneUrl) {
      await cloneRepository(gitCloneUrl, gitBranch, workspaceDir)
    }

    // Run setup commands from REPO_CONFIG (if provided)
    const setupCommands = repoConfig?.codeExecution?.setupCommands
    if (setupCommands && setupCommands.length > 0) {
      logger.info({ setupCommands, workspaceDir }, 'Running setup commands from REPO_CONFIG')
      await runSetupCommands(setupCommands, workspaceDir)
    }

    logger.info({ workspaceDir }, 'Workspace provisioning completed')
    return { status: 'ready', workspaceDir }
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : String(error)
    logger.error({ err: error, workspaceDir }, 'Workspace provisioning failed')
    return { status: 'failed', workspaceDir, error: errorMessage }
  }
}

/**
 * Run setup commands in sequence in the workspace directory
 */
async function runSetupCommands(commands: string[], workspaceDir: string): Promise<void> {
  for (const cmd of commands) {
    logger.info({ command: cmd, workspaceDir }, 'Running setup command')
    try {
      const { stdout, stderr } = await execAsync(cmd, {
        cwd: workspaceDir,
        timeout: 300000, // 5 minute timeout per command
        maxBuffer: 50 * 1024 * 1024, // 50MB buffer
      })

      if (stdout) {
        logger.debug({ stdout: stdout.slice(-1000) }, 'Setup command stdout')
      }
      if (stderr) {
        logger.debug({ stderr: stderr.slice(-1000) }, 'Setup command stderr')
      }

      logger.info({ command: cmd }, 'Setup command completed successfully')
    } catch (error) {
      throw new Error(
        `Setup command failed: ${cmd} - ${error instanceof Error ? error.message : String(error)}`,
      )
    }
  }
}

/**
 * Extract owner/repo from git clone URL.
 * Supports: https://github.com/owner/repo.git, git@github.com:owner/repo.git
 */
function extractRepoPath(cloneUrl: string): string {
  // Handle HTTPS URLs: https://github.com/owner/repo.git
  const httpsMatch = cloneUrl.match(/github\.com[/:]([^/]+\/[^/]+?)(?:\.git)?$/)
  if (httpsMatch) {
    return httpsMatch[1]
  }

  // Fallback: return as-is (gh can handle full URLs too)
  return cloneUrl
}

/**
 * Clones a git repository to the workspace directory using gh CLI.
 * Uses GITHUB_TOKEN environment variable for authentication (set by backend provisioner).
 */
async function cloneRepository(
  gitCloneUrl: string,
  gitBranch: string | undefined,
  workspaceDir: string,
): Promise<void> {
  // Check if directory already has content (might already be cloned)
  const hasGitDir = await directoryExists(`${workspaceDir}/.git`)
  if (hasGitDir) {
    logger.info({ workspaceDir }, 'Git repository already exists, skipping clone')
    return
  }

  // GITHUB_TOKEN should always be present (enforced by backend provisioner)
  const hasGithubToken = !!process.env['GITHUB_TOKEN']
  if (!hasGithubToken) {
    throw new Error('GITHUB_TOKEN environment variable is required for cloning')
  }

  logger.info({ gitCloneUrl, gitBranch, workspaceDir }, 'Cloning repository with gh CLI')

  // Extract owner/repo from clone URL for gh CLI
  const repoPath = extractRepoPath(gitCloneUrl)

  // Use gh CLI - automatically uses GITHUB_TOKEN env var
  // The -- separator passes flags to the underlying git clone command
  const branchArg = gitBranch ? `--branch ${gitBranch}` : ''
  const cloneCommand = `gh repo clone ${repoPath} ${workspaceDir} -- --depth 1 ${branchArg}`

  try {
    const { stdout, stderr } = await execAsync(cloneCommand, {
      timeout: 120000, // 2 minute timeout for clone
    })

    if (stdout) {
      logger.debug({ stdout }, 'gh clone stdout')
    }
    if (stderr) {
      logger.debug({ stderr }, 'gh clone stderr')
    }

    logger.info({ workspaceDir }, 'Repository cloned successfully')
  } catch (error) {
    throw new Error(
      `Failed to clone repository: ${error instanceof Error ? error.message : String(error)}`,
    )
  }
}

/**
 * Check if a directory exists
 */
async function directoryExists(path: string): Promise<boolean> {
  try {
    await access(path)
    return true
  } catch {
    return false
  }
}
