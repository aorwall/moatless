import type { RemoteRepository, RepositoryConfig, TemplateRepository } from '@moatless/types'

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
 * Get the display location for a repository (remote URL)
 */
export function getRepositoryLocation(repo: RepositoryConfig): string {
  // Both remote and template repositories use remote.url
  return repo.remote.url
}
