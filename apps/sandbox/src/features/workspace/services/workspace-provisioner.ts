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
 */
export async function provisionWorkspace(config: SandboxConfig): Promise<ProvisioningResult> {
  const { workspaceDir, gitCloneUrl, gitBranch } = config

  logger.info({ workspaceDir, gitCloneUrl, gitBranch }, 'Starting workspace provisioning')

  try {
    // Ensure workspace directory exists
    await mkdir(workspaceDir, { recursive: true })

    // Clone repository if configured
    if (gitCloneUrl) {
      await cloneRepository(gitCloneUrl, gitBranch, workspaceDir)
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
 * Clones a git repository to the workspace directory.
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

  logger.info({ gitCloneUrl, gitBranch, workspaceDir }, 'Cloning repository')

  // Build clone command
  const branchArg = gitBranch ? `--branch ${gitBranch}` : ''
  const cloneCommand = `git clone ${branchArg} --depth 1 ${gitCloneUrl} ${workspaceDir}`

  try {
    const { stdout, stderr } = await execAsync(cloneCommand, {
      timeout: 120000, // 2 minute timeout for clone
    })

    if (stdout) {
      logger.debug({ stdout }, 'Git clone stdout')
    }
    if (stderr) {
      logger.debug({ stderr }, 'Git clone stderr')
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
