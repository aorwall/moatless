import { exec } from 'node:child_process'
import { promisify } from 'node:util'
import { ValidationError } from '@shared/constants/errors.js'

// TODO: Import from shared types when RemoteRepository is added
// import type { RemoteRepository } from '../../shared/types/index.js'

// Temporary interface until RemoteRepository is added to shared types
interface RemoteRepository {
  url?: string
  token?: string
}

const execAsync = promisify(exec)

// TODO: Update parameter type when RemoteRepository is added to shared types
export async function validateRepositoryAccess(repo: RemoteRepository): Promise<void> {
  try {
    // For GitHub/GitLab, we can use git ls-remote to check accessibility
    const command = `git ls-remote --heads "${repo.url}"`

    const env = { ...process.env }
    if ('token' in repo && repo.token) {
      // Add token to URL for authentication
      const url = new URL(repo.url!)
      url.username = 'token'
      url.password = repo.token
      const authenticatedCommand = `git ls-remote --heads "${url.toString()}"`
      await execAsync(authenticatedCommand, { env, timeout: 10000 })
    } else {
      await execAsync(command, { env, timeout: 10000 })
    }
  } catch (error) {
    throw new ValidationError(
      `Cannot access remote repository: ${error instanceof Error ? error.message : String(error)}`,
      { url: repo.url },
    )
  }
}

export async function getRemoteRepositoryInfo(repo: RemoteRepository): Promise<{
  branches: string[]
  defaultBranch: string
  lastCommit?: string
}> {
  try {
    const command = `git ls-remote --heads --symref "${repo.url}"`
    const { stdout } = await execAsync(command, { timeout: 10000 })

    const lines = stdout.trim().split('\n')
    const branches: string[] = []
    let defaultBranch = 'main'

    for (const line of lines) {
      if (line.startsWith('ref: refs/heads/')) {
        // This is the default branch reference
        defaultBranch = line.split('/').pop() || 'main'
      } else if (line.includes('refs/heads/')) {
        // This is a branch
        const branchName = line.split('refs/heads/')[1]
        if (branchName) {
          branches.push(branchName)
        }
      }
    }

    return {
      branches,
      defaultBranch,
    }
  } catch (error) {
    throw new ValidationError(
      `Cannot get repository info: ${error instanceof Error ? error.message : String(error)}`,
      { url: repo.url },
    )
  }
}

export function parseGitHubUrl(url: string): {
  owner: string
  repo: string
  isValid: boolean
} {
  try {
    const parsed = new URL(url)
    if (parsed.hostname !== 'github.com') {
      return { owner: '', repo: '', isValid: false }
    }

    const pathParts = parsed.pathname.split('/').filter((p) => p)
    if (pathParts.length < 2) {
      return { owner: '', repo: '', isValid: false }
    }

    return {
      owner: pathParts[0],
      repo: pathParts[1].replace(/\.git$/, ''),
      isValid: true,
    }
  } catch {
    return { owner: '', repo: '', isValid: false }
  }
}

export function parseGitLabUrl(url: string): {
  namespace: string
  project: string
  isValid: boolean
} {
  try {
    const parsed = new URL(url)
    if (!parsed.hostname.includes('gitlab')) {
      return { namespace: '', project: '', isValid: false }
    }

    const pathParts = parsed.pathname.split('/').filter((p) => p)
    if (pathParts.length < 2) {
      return { namespace: '', project: '', isValid: false }
    }

    return {
      namespace: pathParts[0],
      project: pathParts[1].replace(/\.git$/, ''),
      isValid: true,
    }
  } catch {
    return { namespace: '', project: '', isValid: false }
  }
}
