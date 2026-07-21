import type { RepositoryResponse } from '@moatless/api-client'

/**
 * Get the display location for a repository (remote URL)
 */
export function getRepositoryLocation(repo: RepositoryResponse): string {
  return repo.remoteUrl ?? ''
}
