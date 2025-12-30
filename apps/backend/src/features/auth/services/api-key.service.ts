import { createHash, randomBytes } from 'node:crypto'
import { inject, injectable } from 'inversify'
import { logger } from '../../../utils/logger'
import { ApiKeyRepository } from '../db/api-key.repository'
import type { ApiKey, ApiKeyCreateResult, ApiKeyPayload } from '../db/api-key.schema'
import { UserRepository } from '../db/user.repository'

/**
 * Service for API key management.
 * Handles key generation, validation, and lifecycle.
 */
@injectable()
export class ApiKeyService {
  private static readonly KEY_PREFIX = 'mvk_'
  private static readonly KEY_BYTES = 32 // 256 bits

  constructor(
    @inject(ApiKeyRepository) private apiKeyRepository: ApiKeyRepository,
    @inject(UserRepository) private userRepository: UserRepository,
  ) {}

  /**
   * Generate a new API key for a user.
   *
   * @param userId - User identifier
   * @param name - Human-readable name for the key
   * @param scopes - Optional array of scopes/permissions
   * @param expiresAt - Optional expiration date
   * @param groupId - Optional group ID for group-based access control
   * @returns The created key result (full key is only returned once!)
   */
  async generateKey(
    userId: string,
    name: string,
    scopes?: string[],
    expiresAt?: Date,
    groupId?: string,
  ): Promise<ApiKeyCreateResult> {
    // Generate random key
    const keyBytes = randomBytes(ApiKeyService.KEY_BYTES)
    const keyBase64 = keyBytes.toString('base64url')
    const fullKey = `${ApiKeyService.KEY_PREFIX}${keyBase64}`

    // Hash the key for storage
    const keyHash = this.hashKey(fullKey)

    // Generate prefix for identification (prefix + first 8 chars of the random part)
    const keyPrefix = `${ApiKeyService.KEY_PREFIX}${keyBase64.slice(0, 8)}`

    // Generate unique ID
    const id = randomBytes(16).toString('hex')

    const now = new Date()

    await this.apiKeyRepository.createApiKey({
      id,
      userId,
      name,
      keyHash,
      keyPrefix,
      scopes: scopes ? JSON.stringify(scopes) : null,
      groupId: groupId ?? null,
      lastUsedAt: null,
      expiresAt: expiresAt ?? null,
      createdAt: now,
    })

    logger.info({ userId, keyId: id, keyPrefix, name }, 'Created API key')

    return {
      id,
      key: fullKey,
      name,
      keyPrefix,
      scopes: scopes ?? null,
      expiresAt: expiresAt ?? null,
      createdAt: now,
    }
  }

  /**
   * Validate an API key and return the payload if valid.
   *
   * @param key - The full API key to validate
   * @returns Key payload if valid, null otherwise
   */
  async validateKey(key: string): Promise<ApiKeyPayload | null> {
    // Check prefix
    if (!key.startsWith(ApiKeyService.KEY_PREFIX)) {
      return null
    }

    // Hash the key and look it up
    const keyHash = this.hashKey(key)
    const keyRow = await this.apiKeyRepository.getApiKeyByHash(keyHash)

    if (!keyRow) {
      logger.debug({ keyPrefix: key.slice(0, 12) }, 'API key not found')
      return null
    }

    // Check expiration
    if (keyRow.expiresAt && new Date() > keyRow.expiresAt) {
      logger.debug({ keyId: keyRow.id }, 'API key expired')
      return null
    }

    // Update last used timestamp (fire and forget)
    this.apiKeyRepository.updateLastUsed(keyRow.id).catch((err) => {
      logger.warn({ err, keyId: keyRow.id }, 'Failed to update API key last used timestamp')
    })

    // Get user info
    const user = await this.userRepository.getUser(keyRow.userId)

    return {
      keyId: keyRow.id,
      userId: keyRow.userId,
      userName: user?.login ?? null,
      scopes: keyRow.scopes ? JSON.parse(keyRow.scopes) : null,
      groupId: keyRow.groupId ?? null,
      expiresAt: keyRow.expiresAt ?? null,
    }
  }

  /**
   * List all API keys for a user.
   * Note: Does not include the actual key (hash is not reversible).
   *
   * @param userId - User identifier
   * @returns Array of API keys (without the key hash)
   */
  listKeys(userId: string): Promise<ApiKey[]> {
    return this.apiKeyRepository.getApiKeysByUserId(userId)
  }

  /**
   * Revoke (delete) an API key.
   * Only the owner can revoke their keys.
   *
   * @param keyId - API key identifier
   * @param userId - User identifier (for authorization)
   */
  async revokeKey(keyId: string, userId: string): Promise<boolean> {
    // First check if the key exists and belongs to the user
    const key = await this.apiKeyRepository.getApiKeyById(keyId)

    if (!key) {
      logger.debug({ keyId }, 'API key not found for revocation')
      return false
    }

    if (key.userId !== userId) {
      logger.warn({ keyId, userId, keyOwner: key.userId }, 'User tried to revoke another user key')
      return false
    }

    await this.apiKeyRepository.deleteApiKey(keyId)
    logger.info({ keyId, userId }, 'Revoked API key')
    return true
  }

  /**
   * Hash an API key using SHA-256.
   */
  private hashKey(key: string): string {
    return createHash('sha256').update(key).digest('hex')
  }
}
