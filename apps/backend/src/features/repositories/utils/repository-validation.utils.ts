import { promises as fs } from 'node:fs'
import path from 'node:path'
import { ValidationError } from '@moatless/types'

/**
 * Validate that a path exists and is a git repository
 */
export async function validateLocalRepository(repoPath: string): Promise<void> {
  try {
    const stats = await fs.stat(repoPath)
    if (!stats.isDirectory()) {
      throw new ValidationError(`Path ${repoPath} is not a directory`, {
        path: repoPath,
      })
    }

    // Check if it's a git repository
    const gitPath = path.join(repoPath, '.git')
    try {
      await fs.access(gitPath)
    } catch {
      throw new ValidationError(`Path ${repoPath} is not a git repository`, {
        path: repoPath,
      })
    }
  } catch (error) {
    if (error instanceof ValidationError) {
      throw error
    }
    throw new ValidationError(
      `Cannot access repository at ${repoPath}: ${error instanceof Error ? error.message : String(error)}`,
      {
        path: repoPath,
        error: error instanceof Error ? error.message : String(error),
      },
    )
  }
}

/**
 * Parse a GitHub HTTPS URL and extract owner/repo information
 * Only accepts GitHub HTTPS URLs for security
 */
export function parseGitUrl(remoteUrl: string): {
  owner: string
  repoName: string
} {
  // Only accept GitHub HTTPS URLs
  const githubHttpsPattern = /^https:\/\/github\.com\/([^/]+)\/([^/.]+)(?:\.git)?$/

  const match = remoteUrl.match(githubHttpsPattern)
  if (match) {
    return {
      owner: match[1] || '',
      repoName: match[2] || '',
    }
  }

  throw new ValidationError(
    'Invalid GitHub URL format. Only HTTPS URLs are supported (e.g., https://github.com/owner/repo)',
    { url: remoteUrl },
  )
}

/**
 * Generate a unique clone path by adding numeric suffix if directory exists
 */
export async function generateUniqueClonePath(
  baseDir: string,
  baseName: string,
): Promise<{ path: string; name: string }> {
  const basePath = path.join(baseDir, baseName)

  try {
    await fs.access(basePath)
    // Directory exists, try with numeric suffixes
    let counter = 2
    while (true) {
      const uniqueName = `${baseName}-${counter}`
      const uniquePath = path.join(baseDir, uniqueName)

      try {
        await fs.access(uniquePath)
        // This path also exists, try next number
        counter++
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
          // Path doesn't exist, we can use it
          return { path: uniquePath, name: uniqueName }
        }
        throw error
      }
    }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
      // Base path doesn't exist, we can use it
      return { path: basePath, name: baseName }
    }
    throw error
  }
}
