import type { RepositoryConfig } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { Storage } from '../../../infrastructure/storage/storage.js'
import { StorageSymbol } from '../../../storage/storage-backend.js'

/**
 * Feature-specific storage adapter for repositories.
 * Wraps the Storage interface with type-safe methods and key structure.
 */
@injectable()
export class RepositoryStorage {
  constructor(@inject(StorageSymbol) private readonly storage: Storage) {}

  async save(repository: RepositoryConfig): Promise<void> {
    await this.storage.save(this.key(repository.id), repository)
  }

  get(id: string): Promise<RepositoryConfig | null> {
    return this.storage.get<RepositoryConfig>(this.key(id))
  }

  async list(): Promise<string[]> {
    // Returns keys like "repositories/id", we'll need to extract IDs
    const keys = await this.storage.list('repositories/')
    return keys.map((key) => key.replace('repositories/', ''))
  }

  async delete(id: string): Promise<void> {
    await this.storage.delete(this.key(id))
  }

  exists(id: string): Promise<boolean> {
    return this.storage.exists(this.key(id))
  }

  private key(id: string): string {
    return `repositories/${id}`
  }
}
