import path from 'node:path'
import type { RepositoryConfig, RepositoryConfigFile } from '@moatless/types'
import { RepositoryConfigFileSchema, ValidationError } from '@moatless/types'
import { pathExists, readFileSafe, writeFileSafe } from '../../../utils/filesystem'
import { logger } from '../../../utils/logger'

/**
 * Detect if a repository has a .moatless/config.json file
 */
export async function detectRepoConfigFile(repoPath: string): Promise<boolean> {
  const configPath = path.join(repoPath, '.moatless', 'config.json')
  return await pathExists(configPath)
}

/**
 * Read repository configuration from .moatless/config.json file
 * Tries local path first, falls back to GitHub API if remote URL is provided
 */
export async function readRepoConfigFile(
  repoPath: string,
  options?: {
    remoteUrl?: string
    githubToken?: string
    githubService?: {
      getFileContent: (
        token: string,
        owner: string,
        repo: string,
        path: string,
      ) => Promise<string | null>
    }
    parseGitUrl?: (url: string) => { owner: string; repoName: string }
  },
): Promise<RepositoryConfigFile | null> {
  const configPath = path.join(repoPath, '.moatless', 'config.json')

  // Try reading from local path first
  let content = await readFileSafe(configPath)

  // If not found locally and remoteUrl is available, try GitHub API
  if (!content && options?.remoteUrl && options?.githubToken) {
    if (!(options.githubService && options.parseGitUrl)) {
      logger.debug(
        {
          path: configPath,
        },
        'GitHub service or parseGitUrl not provided, skipping remote config fetch',
      )
      return null
    }

    try {
      const parsedUrl = options.parseGitUrl(options.remoteUrl)
      content = await options.githubService.getFileContent(
        options.githubToken,
        parsedUrl.owner,
        parsedUrl.repoName,
        '.moatless/config.json',
      )
    } catch (error) {
      logger.debug(
        {
          remoteUrl: options.remoteUrl,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to read config file from GitHub',
      )
    }
  }

  if (!content) {
    return null
  }

  // Parse and validate the config file
  try {
    const json = JSON.parse(content)
    const validated = RepositoryConfigFileSchema.parse(json)
    return validated
  } catch (error) {
    logger.warn(
      {
        path: configPath,
        remoteUrl: options?.remoteUrl,
        error: error instanceof Error ? error.message : String(error),
      },
      'Invalid repository config file format',
    )
    return null
  }
}

/**
 * Write repository configuration to .moatless/config.json file
 */
export async function writeRepoConfigFile(
  repoPath: string,
  config: RepositoryConfigFile,
): Promise<void> {
  const configPath = path.join(repoPath, '.moatless', 'config.json')

  try {
    const content = JSON.stringify(config, null, 2)
    await writeFileSafe(configPath, content)
    logger.info({ path: configPath }, 'Created repository config file')
  } catch (error) {
    logger.error({ err: error as Error }, 'Failed to write repository config file')
    throw new ValidationError('Failed to create .moatless/config.json file', error)
  }
}

/**
 * Merge repository configuration with config file
 * Always reads and merges config file if present
 */
export async function mergeWithRepoConfig(
  repository: RepositoryConfig,
  options?: {
    remoteUrl?: string
    githubToken?: string
    githubService?: {
      getFileContent: (
        token: string,
        owner: string,
        repo: string,
        path: string,
      ) => Promise<string | null>
    }
    parseGitUrl?: (url: string) => { owner: string; repoName: string }
  },
): Promise<RepositoryConfig> {
  // Remote and template repositories need GitHub API access to read config
  if (repository.kind === 'remote' || repository.kind === 'template') {
    // For now, skip config sync for remote/template repositories
    // TODO: Implement GitHub API-based config file reading
    logger.debug(
      {
        repositoryId: repository.id,
      },
      'Config sync not yet supported for remote/template repositories',
    )
    return repository
  }

  // Only local repositories have a local path
  const configFile = await readRepoConfigFile(repository.path, {
    ...(options?.remoteUrl && { remoteUrl: options.remoteUrl }),
    ...(options?.githubToken && { githubToken: options.githubToken }),
    ...(options?.githubService && { githubService: options.githubService }),
    ...(options?.parseGitUrl && { parseGitUrl: options.parseGitUrl }),
  })

  if (!configFile) {
    // No config file found, return repository as-is
    return repository
  }

  // Merge config file values into repository
  return {
    ...repository,
    ...(configFile.systemPrompt !== undefined && { systemPrompt: configFile.systemPrompt }),
    ...(configFile.codeExecution !== undefined && { codeExecution: configFile.codeExecution }),
    ...(configFile.previewConfig !== undefined && { previewConfig: configFile.previewConfig }),
  }
}
