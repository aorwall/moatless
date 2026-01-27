import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { inject, injectable } from 'inversify'
import { SettingsService } from '../../../services/settings.service'
import { getDataDir } from '../../../utils/data-dir'
import { logger } from '../../../utils/logger'
import { LocalStorageBackend } from '../backends/local-storage-backend'
import { S3StorageBackend } from '../backends/s3-storage-backend'
import type { StorageBackend } from '../backends/storage-backend'

export interface UploadedFile {
  fileId: string
  filename: string
  mimeType: string
  size: number
  uploadedAt: string
}

export interface FileWithMetadata {
  buffer: Buffer
  metadata: UploadedFile
}

/**
 * FileStorageService - Manages file uploads with pluggable storage backends
 *
 * Storage backends:
 * - Local: Files stored on filesystem (Docker mode)
 * - S3: Files stored in S3 (Kubernetes mode)
 *
 * Files are accessible to sandbox containers via volume mounts:
 * - Docker: Host uploads directory mounted to /mnt/uploads
 * - K8s: S3 bucket mounted via CSI driver to /mnt/uploads
 *
 * Metadata is always stored locally for quick access.
 */
@injectable()
export class FileStorageService {
  private readonly metadataDir: string
  private readonly localUploadDir: string
  private storageBackend: StorageBackend | null = null
  private readonly log = logger.child({ component: 'FileStorageService' })

  constructor(@inject(SettingsService) private readonly settingsService: SettingsService) {
    const dataDir = getDataDir()
    this.localUploadDir = process.env['UPLOAD_DIR'] || path.join(dataDir, 'uploads')
    this.metadataDir = path.join(this.localUploadDir, '.metadata')
  }

  /**
   * Get or create the storage backend based on settings
   */
  private getBackend(): StorageBackend {
    if (this.storageBackend) {
      return this.storageBackend
    }

    const storageSettings = this.settingsService.getStorageSettings()

    if (storageSettings.backend === 's3') {
      if (!storageSettings.s3BucketName) {
        throw new Error('S3_BUCKET_NAME is required when FILE_STORAGE_BACKEND=s3')
      }
      this.storageBackend = new S3StorageBackend(
        storageSettings.s3BucketName,
        storageSettings.s3KeyPrefix,
        storageSettings.s3Region,
      )
      this.log.info(
        {
          backend: 's3',
          bucket: storageSettings.s3BucketName,
          prefix: storageSettings.s3KeyPrefix,
        },
        'Using S3 storage backend',
      )
    } else {
      this.storageBackend = new LocalStorageBackend(this.localUploadDir)
      this.log.info(
        { backend: 'local', uploadDir: this.localUploadDir },
        'Using local storage backend',
      )
    }

    return this.storageBackend
  }

  /**
   * Upload a file and store its metadata
   */
  async uploadFile(buffer: Buffer, filename: string, mimeType: string): Promise<UploadedFile> {
    const fileId = randomUUID()

    // Validate filename length (255 chars is typical filesystem limit)
    if (filename.length > 255) {
      throw new Error('Filename too long (max 255 characters)')
    }

    // Ensure metadata directory exists (always local)
    await fs.mkdir(this.metadataDir, { recursive: true })

    // Store file using the configured backend
    const backend = this.getBackend()
    await backend.write(fileId, buffer)

    // Create metadata
    const metadata: UploadedFile = {
      fileId,
      filename,
      mimeType,
      size: buffer.length,
      uploadedAt: new Date().toISOString(),
    }

    // Store metadata locally
    const metadataPath = path.join(this.metadataDir, `${fileId}.json`)
    await fs.writeFile(metadataPath, JSON.stringify(metadata, null, 2))

    this.log.info(
      {
        fileId,
        filename,
        size: buffer.length,
        mimeType,
      },
      'File uploaded successfully',
    )

    return metadata
  }

  /**
   * Get file content and metadata
   */
  async getFile(fileId: string): Promise<FileWithMetadata> {
    // Read metadata first to validate file exists
    const metadata = await this.getFileMetadata(fileId)

    // Read file content using the configured backend
    const backend = this.getBackend()
    try {
      const buffer = await backend.read(fileId)
      return { buffer, metadata }
    } catch (error) {
      this.log.error(
        {
          fileId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to read file',
      )
      throw new Error(`File not found: ${fileId}`)
    }
  }

  /**
   * Get file metadata without loading content
   */
  async getFileMetadata(fileId: string): Promise<UploadedFile> {
    const metadataPath = path.join(this.metadataDir, `${fileId}.json`)

    try {
      const metadataContent = await fs.readFile(metadataPath, 'utf-8')
      const metadata = JSON.parse(metadataContent) as UploadedFile
      return metadata
    } catch (error) {
      this.log.warn(
        {
          fileId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to read file metadata',
      )
      throw new Error(`File not found: ${fileId}`)
    }
  }

  /**
   * Get the sandbox-accessible file path for a given fileId
   *
   * Returns the path where the file will be accessible inside sandbox containers.
   * Both Docker and Kubernetes modes mount uploads to the same path.
   */
  getFilePath(fileId: string): string {
    const storageSettings = this.settingsService.getStorageSettings()
    return path.posix.join(storageSettings.sandboxMountPath, fileId)
  }

  /**
   * Delete a file and its metadata
   */
  async deleteFile(fileId: string): Promise<void> {
    const metadataPath = path.join(this.metadataDir, `${fileId}.json`)

    try {
      // Delete file using the configured backend
      const backend = this.getBackend()
      await backend.delete(fileId)

      // Delete metadata
      await fs.unlink(metadataPath).catch((error) => {
        this.log.warn(
          {
            fileId,
            error: error instanceof Error ? error.message : String(error),
          },
          'Failed to delete metadata (may not exist)',
        )
      })

      this.log.info({ fileId }, 'File deleted successfully')
    } catch (error) {
      this.log.error(
        {
          fileId,
          error: error instanceof Error ? error.message : String(error),
        },
        'Failed to delete file',
      )
      throw error
    }
  }

  /**
   * Check if a file exists
   */
  async fileExists(fileId: string): Promise<boolean> {
    const backend = this.getBackend()
    return await backend.exists(fileId)
  }
}
