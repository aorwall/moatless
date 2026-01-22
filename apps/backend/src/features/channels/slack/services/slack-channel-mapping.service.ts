import type { RepositoryConfig } from '@moatless/types'
import { inject, injectable } from 'inversify'
import { logger } from '../../../../utils/logger'
import { RepositoryRepository } from '../../../repositories/db/repository.repository'
import { SlackChannelMappingRepository } from '../../db/slack-channel-mapping.repository'
import type { SlackChannelMapping } from '../../db/slack-channel-mapping.schema'

/**
 * Slack channel mapping with repository details.
 */
export interface SlackChannelMappingWithRepository {
  mapping: SlackChannelMapping
  repository: RepositoryConfig | null
}

/**
 * Service for managing Slack channel-to-repository mappings.
 * Enables users to mention @moatless without specifying repo: if channel has a default.
 */
@injectable()
export class SlackChannelMappingService {
  constructor(
    @inject(SlackChannelMappingRepository)
    private readonly mappingRepository: SlackChannelMappingRepository,
    @inject(RepositoryRepository)
    private readonly repositoryRepository: RepositoryRepository,
  ) {}

  /**
   * Get the default repository for a Slack channel.
   * Returns null if no mapping exists or repository not found.
   *
   * @param channelId - Slack channel ID
   * @param teamId - Slack workspace ID
   * @returns Repository configuration or null
   */
  async getDefaultRepository(channelId: string, teamId: string): Promise<RepositoryConfig | null> {
    const mapping = await this.mappingRepository.findByChannel(channelId, teamId)
    if (!mapping) {
      return null
    }

    // Use getInternal since this may be called from event processing
    // where there's no user session context
    const repository = await this.repositoryRepository.getInternal(mapping.repositoryId)
    if (!repository) {
      logger.warn(
        { channelId, teamId, repositoryId: mapping.repositoryId },
        'Channel mapping points to non-existent repository',
      )
      return null
    }

    return repository
  }

  /**
   * Set the default repository for a Slack channel.
   * Creates a new mapping or updates an existing one.
   *
   * @param channelId - Slack channel ID
   * @param teamId - Slack workspace ID
   * @param repositoryId - Repository ID to set as default
   * @param createdBy - User ID creating the mapping
   * @returns Created or updated mapping
   */
  async setDefaultRepository(
    channelId: string,
    teamId: string,
    repositoryId: string,
    createdBy: string,
  ): Promise<SlackChannelMapping> {
    // Check if repository exists
    const repository = await this.repositoryRepository.getInternal(repositoryId)
    if (!repository) {
      throw new Error(`Repository not found: ${repositoryId}`)
    }

    // Check if mapping already exists
    const existing = await this.mappingRepository.findByChannel(channelId, teamId)
    if (existing) {
      // Update existing mapping
      const updated = await this.mappingRepository.update(channelId, teamId, { repositoryId })
      if (!updated) {
        throw new Error('Failed to update channel mapping')
      }
      logger.info(
        { channelId, teamId, repositoryId, previousRepositoryId: existing.repositoryId },
        'Updated Slack channel mapping',
      )
      return updated
    }

    // Create new mapping
    const now = new Date()
    const mapping = await this.mappingRepository.create({
      channelId,
      teamId,
      repositoryId,
      createdBy,
      createdAt: now,
      updatedAt: now,
    })

    logger.info({ channelId, teamId, repositoryId, createdBy }, 'Created Slack channel mapping')

    return mapping
  }

  /**
   * Remove the default repository mapping for a Slack channel.
   *
   * @param channelId - Slack channel ID
   * @param teamId - Slack workspace ID
   * @returns True if mapping was deleted, false if not found
   */
  async removeDefaultRepository(channelId: string, teamId: string): Promise<boolean> {
    const deleted = await this.mappingRepository.delete(channelId, teamId)
    if (deleted) {
      logger.info({ channelId, teamId }, 'Removed Slack channel mapping')
    }
    return deleted
  }

  /**
   * Get the mapping for a specific channel.
   *
   * @param channelId - Slack channel ID
   * @param teamId - Slack workspace ID
   * @returns Mapping with repository details or null
   */
  async getMapping(
    channelId: string,
    teamId: string,
  ): Promise<SlackChannelMappingWithRepository | null> {
    const mapping = await this.mappingRepository.findByChannel(channelId, teamId)
    if (!mapping) {
      return null
    }

    const repository = await this.repositoryRepository.getInternal(mapping.repositoryId)
    return { mapping, repository }
  }

  /**
   * List all Slack channel mappings with repository details.
   *
   * @returns Array of mappings with repository details
   */
  async listMappings(): Promise<SlackChannelMappingWithRepository[]> {
    const mappings = await this.mappingRepository.list()
    const result: SlackChannelMappingWithRepository[] = []

    for (const mapping of mappings) {
      const repository = await this.repositoryRepository.getInternal(mapping.repositoryId)
      result.push({ mapping, repository })
    }

    return result
  }

  /**
   * Check if a user is authorized to modify a channel mapping.
   * Only repository owners can create/update/delete mappings for their repos.
   *
   * @param repositoryId - Repository ID to check
   * @param userId - User ID to check authorization for
   * @returns True if user is authorized
   */
  async isAuthorized(repositoryId: string, userId: string): Promise<boolean> {
    const repository = await this.repositoryRepository.getInternal(repositoryId)
    if (!repository) {
      return false
    }

    // Only repository owner can manage mappings
    return repository.createdBy === userId
  }
}
