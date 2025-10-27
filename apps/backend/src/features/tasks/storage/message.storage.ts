import type { AgentMessage } from '@moatless/types'
import { inject, injectable } from 'inversify'
import type { StorageBackend } from '../../../storage/storage-backend'
import { StorageSymbol } from '../../../storage/storage-backend'

/**
 * Storage adapter for task message persistence.
 * Uses the legacy StorageBackend interface for JSONL operations.
 * Messages are stored as append-only logs at key: messages/{taskId}
 *
 * Note: This temporarily uses StorageBackend instead of the infrastructure Storage
 * interface because JSONL operations (readJsonl/appendJsonl) are not yet available
 * in the minimal Storage interface. This will be refactored in a future phase.
 */
@injectable()
export class MessageStorage {
  constructor(@inject(StorageSymbol) private readonly storage: StorageBackend) {}

  /**
   * Read all messages for a task from the JSONL log
   * @param taskId - Task identifier
   * @returns Array of agent messages (empty array if file doesn't exist)
   */
  async read(taskId: string): Promise<AgentMessage[]> {
    try {
      return await this.storage.readJsonl<AgentMessage>(this.getKey(taskId))
    } catch (_error) {
      // Return empty array if file doesn't exist or can't be read
      return []
    }
  }

  /**
   * Append a message to the task's JSONL log
   * @param taskId - Task identifier
   * @param message - Agent message to append
   */
  async append(taskId: string, message: AgentMessage): Promise<void> {
    await this.storage.appendJsonl(this.getKey(taskId), message)
  }

  /**
   * Delete all messages for a task
   * @param taskId - Task identifier
   */
  async delete(taskId: string): Promise<void> {
    await this.storage.delete(this.getKey(taskId))
  }

  /**
   * Check if messages exist for a task
   * @param taskId - Task identifier
   * @returns True if message log exists
   */
  async exists(taskId: string): Promise<boolean> {
    return await this.storage.exists(this.getKey(taskId))
  }

  /**
   * Generate storage key for task messages
   * @param taskId - Task identifier
   * @returns Storage key
   */
  private getKey(taskId: string): string {
    return `messages/${taskId}`
  }
}
