import type { FigmaImportMetadata, FigmaPKCEVerifier, FigmaUserConfig } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { Storage } from '../../../infrastructure/storage/storage'
import { InfrastructureStorageSymbol } from '../../../infrastructure/storage/storage'

/**
 * Storage adapter for Figma feature persistence.
 * Uses the infrastructure Storage interface to persist Figma-related data.
 */
@injectable()
export class FigmaStorage {
  constructor(@inject(InfrastructureStorageSymbol) private readonly storage: Storage) {}

  /**
   * Save Figma import metadata
   * @param metadata - Figma import metadata to save
   */
  async saveFigmaImport(metadata: FigmaImportMetadata): Promise<void> {
    await this.storage.save(this.getFigmaImportKey(metadata.importId), metadata)
  }

  /**
   * Get Figma import metadata by ID
   * @param importId - Import identifier
   * @returns Import metadata or null if not found
   */
  async getFigmaImport(importId: string): Promise<FigmaImportMetadata | null> {
    return await this.storage.get<FigmaImportMetadata>(this.getFigmaImportKey(importId))
  }

  /**
   * Save PKCE verifier data for OAuth flow
   * @param state - OAuth state parameter
   * @param data - PKCE verifier data
   */
  async savePKCEVerifier(state: string, data: FigmaPKCEVerifier): Promise<void> {
    await this.storage.save(this.getPKCEVerifierKey(state), data)
  }

  /**
   * Get PKCE verifier data by state
   * @param state - OAuth state parameter
   * @returns PKCE verifier data or null if not found
   */
  async getPKCEVerifier(state: string): Promise<FigmaPKCEVerifier | null> {
    return await this.storage.get<FigmaPKCEVerifier>(this.getPKCEVerifierKey(state))
  }

  /**
   * Delete PKCE verifier data
   * @param state - OAuth state parameter
   */
  async deletePKCEVerifier(state: string): Promise<void> {
    await this.storage.delete(this.getPKCEVerifierKey(state))
  }

  /**
   * Save user's Figma configuration
   * @param userId - User identifier
   * @param config - Figma user configuration
   */
  async saveUserConfig(userId: string, config: FigmaUserConfig): Promise<void> {
    await this.storage.save(this.getUserConfigKey(userId), config)
  }

  /**
   * Get user's Figma configuration
   * @param userId - User identifier
   * @returns Figma user configuration or null if not found
   */
  async getUserConfig(userId: string): Promise<FigmaUserConfig | null> {
    return await this.storage.get<FigmaUserConfig>(this.getUserConfigKey(userId))
  }

  /**
   * Delete user's Figma configuration
   * @param userId - User identifier
   */
  async deleteUserConfig(userId: string): Promise<void> {
    await this.storage.delete(this.getUserConfigKey(userId))
  }

  /**
   * Generate storage key for Figma import metadata
   * @param importId - Import identifier
   * @returns Storage key
   */
  private getFigmaImportKey(importId: string): string {
    return `figma-imports/${importId}`
  }

  /**
   * Generate storage key for PKCE verifier
   * @param state - OAuth state parameter
   * @returns Storage key
   */
  private getPKCEVerifierKey(state: string): string {
    return `figma-oauth-verifiers/${state}`
  }

  /**
   * Generate storage key for user Figma configuration
   * @param userId - User identifier
   * @returns Storage key
   */
  private getUserConfigKey(userId: string): string {
    return `users/${userId}/figma`
  }
}
