import type { RepositoryResponse } from '@moatless/api-client'

/**
 * Type guard to check if a repository is a remote repository
 */
export function isRemoteRepository(repo: RepositoryResponse): boolean {
  return repo.kind === 'remote'
}

/**
 * Type guard to check if a repository is a template repository
 */
export function isTemplateRepository(repo: RepositoryResponse): boolean {
  return repo.kind === 'template'
}

/**
 * Get the display location for a repository (remote URL)
 */
export function getRepositoryLocation(repo: RepositoryResponse): string {
  return repo.remoteUrl ?? ''
}
