import type { Container } from 'inversify'
import { logger } from '../../../utils/logger'
import { RepositoryRepository } from '../db/repository.repository'
import { RepositoryService } from '../services/repository.service'

const DEFAULT_TEMPLATE_URL = 'https://github.com/aorwall/moatable.git'
const DEFAULT_TEMPLATE_NAME = 'moatable'

/**
 * Seed default template repositories on first startup.
 * Creates the moatable template if it doesn't exist.
 *
 * This is idempotent - safe to run multiple times.
 */
export async function seedDefaultRepositories(container: Container): Promise<void> {
  const repositoryRepository = container.get(RepositoryRepository)
  const repositoryService = container.get(RepositoryService)

  // Check if moatable template already exists
  const existing = await repositoryRepository.findByRemoteUrl(DEFAULT_TEMPLATE_URL)
  if (existing) {
    logger.debug({ url: DEFAULT_TEMPLATE_URL }, 'Default template already exists, skipping seed')
    return
  }

  try {
    // Create moatable as a template repository
    // Config file will be read automatically by createTemplateRepository
    const template = await repositoryService.createTemplateRepository({
      kind: 'template',
      name: DEFAULT_TEMPLATE_NAME,
      remoteUrl: DEFAULT_TEMPLATE_URL,
      visibility: 'public',
    })

    logger.info(
      { repoId: template.id, name: template.name, url: DEFAULT_TEMPLATE_URL },
      'Seeded default template repository',
    )
  } catch (error) {
    // Log error but don't fail startup - seeding is best-effort
    logger.error(
      { err: error, url: DEFAULT_TEMPLATE_URL },
      'Failed to seed default template repository',
    )
  }
}
