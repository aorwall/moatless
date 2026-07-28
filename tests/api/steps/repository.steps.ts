/**
 * Repository creation step.
 *
 * Applies the defaults a chain does not care about (a remote GitHub repository)
 * and returns the id alongside the response. Every other repository call a chain
 * makes against `api` directly.
 */

import type { MoatApi } from '../../shared/moat-api'
import type { RepositoryResponse } from '../../shared/types'

// =============================================================================
// REPO-01: Create repository from GitHub URL
// =============================================================================

export async function createRepository(
  api: MoatApi,
  config: {
    name: string
    remoteUrl: string
    kind?: 'remote' | 'template' | 'git'
    provider?: 'github' | 'gitlab' | 'gitness'
    description?: string
  },
): Promise<{ repositoryId: string; repository: RepositoryResponse }> {
  const repository = await api.createRepository({
    name: config.name,
    kind: config.kind ?? 'remote',
    remoteUrl: config.remoteUrl,
    provider: config.provider ?? 'github',
    description: config.description,
  })
  return { repositoryId: repository.id, repository }
}
