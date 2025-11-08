import type {
  LocalRepository,
  RemoteRepository,
  RepositoryConfig,
  TemplateRepository,
} from '@moatless/types'

/**
 * Type guard to check if a repository is a local repository
 */
export function isLocalRepository(repo: RepositoryConfig): repo is LocalRepository {
  return repo.kind === 'local'
}

/**
 * Type guard to check if a repository is a remote repository
 */
export function isRemoteRepository(repo: RepositoryConfig): repo is RemoteRepository {
  return repo.kind === 'remote'
}

/**
 * Type guard to check if a repository is a template repository
 */
export function isTemplateRepository(repo: RepositoryConfig): repo is TemplateRepository {
  return repo.kind === 'template'
}

/**
 * Get the display location for a repository (path or remote URL)
 */
export function getRepositoryLocation(repo: RepositoryConfig): string {
  if (isLocalRepository(repo) || isTemplateRepository(repo)) {
    return repo.path
  }
  if (isRemoteRepository(repo)) {
    return repo.remote.url
  }
  return ''
}

/**
 * Get the display type label for a repository
 */
export function getRepositoryTypeLabel(repo: RepositoryConfig): string {
  if (isLocalRepository(repo)) {
    return 'Local'
  }
  if (isRemoteRepository(repo)) {
    return `Remote (${repo.remote.provider})`
  }
  if (isTemplateRepository(repo)) {
    return 'Template'
  }
  return 'Unknown'
}
