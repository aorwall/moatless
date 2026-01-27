/**
 * StorageBackend interface for file storage abstraction.
 *
 * This interface allows switching between different storage backends
 * (local filesystem, S3) without changing the FileStorageService logic.
 */
export interface StorageBackend {
  /**
   * Write file content to storage
   * @param key - Unique identifier for the file (fileId)
   * @param buffer - File content as Buffer
   */
  write(key: string, buffer: Buffer): Promise<void>

  /**
   * Read file content from storage
   * @param key - Unique identifier for the file (fileId)
   * @returns File content as Buffer
   * @throws Error if file not found
   */
  read(key: string): Promise<Buffer>

  /**
   * Delete file from storage
   * @param key - Unique identifier for the file (fileId)
   */
  delete(key: string): Promise<void>

  /**
   * Check if file exists in storage
   * @param key - Unique identifier for the file (fileId)
   * @returns true if file exists, false otherwise
   */
  exists(key: string): Promise<boolean>
}

/**
 * Symbol for StorageBackend dependency injection
 */
export const StorageBackendSymbol = Symbol.for('StorageBackend')
