import { promises as fs } from 'node:fs'
import path from 'node:path'
import { logger } from '../../../utils/logger'
import type { StorageBackend } from './storage-backend'

/**
 * LocalStorageBackend - Local filesystem storage implementation
 *
 * Stores files on the local filesystem. Used for Docker mode where
 * files can be accessed via volume mounts.
 */
export class LocalStorageBackend implements StorageBackend {
  private readonly log = logger.child({ component: 'LocalStorageBackend' })

  constructor(private readonly uploadDir: string) {}

  async write(key: string, buffer: Buffer): Promise<void> {
    await fs.mkdir(this.uploadDir, { recursive: true })
    const filePath = path.join(this.uploadDir, key)
    await fs.writeFile(filePath, buffer)
    this.log.debug({ key, size: buffer.length }, 'File written to local storage')
  }

  async read(key: string): Promise<Buffer> {
    const filePath = path.join(this.uploadDir, key)
    try {
      const buffer = await fs.readFile(filePath)
      this.log.debug({ key, size: buffer.length }, 'File read from local storage')
      return buffer
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      this.log.error({ key, error: message }, 'Failed to read file from local storage')
      throw new Error(`File not found: ${key}`)
    }
  }

  async delete(key: string): Promise<void> {
    const filePath = path.join(this.uploadDir, key)
    try {
      await fs.unlink(filePath)
      this.log.debug({ key }, 'File deleted from local storage')
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      this.log.warn({ key, error: message }, 'Failed to delete file (may not exist)')
    }
  }

  async exists(key: string): Promise<boolean> {
    const filePath = path.join(this.uploadDir, key)
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }
}
