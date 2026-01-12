import type { RepositoryConfig, RepositoryConfigFile } from '@moatless/types'
import { logger } from '../../../utils/logger'

/**
 * Read repository configuration from GitHub API
 * TODO: Implement GitHub API-based config file reading
 */
export function readRepoConfigFile(
  _repoPath: string,
  _options?: {
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
): RepositoryConfigFile | null {
  // TODO: Implement GitHub API-based config file reading
  // For now, return null as config sync is not yet supported
  return null
}

/**
 * Merge repository configuration with config file
 * For remote/template repositories, GitHub API access is required to read config
 */
export function mergeWithRepoConfig(repository: RepositoryConfig): Promise<RepositoryConfig> {
  // For now, skip config sync for remote/template repositories
  // TODO: Implement GitHub API-based config file reading
  logger.debug(
    {
      repositoryId: repository.id,
    },
    'Config sync not yet supported for remote/template repositories',
  )
  return Promise.resolve(repository)
}
