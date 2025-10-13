import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import { getDataDir } from '../utils/data-dir.js'
import { logger } from '../utils/logger.js'

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

export class UploadService {
  private readonly uploadDir: string
  private readonly metadataDir: string

  constructor() {
    const dataDir = getDataDir()
    this.uploadDir = process.env['UPLOAD_DIR'] || path.join(dataDir, 'uploads')
    this.metadataDir = path.join(this.uploadDir, '.metadata')
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

    // Create directories
    await fs.mkdir(this.uploadDir, { recursive: true })
    await fs.mkdir(this.metadataDir, { recursive: true })

    // Store file without extension (flat structure)
    const filePath = path.join(this.uploadDir, fileId)
    await fs.writeFile(filePath, buffer)

    // Create metadata
    const metadata: UploadedFile = {
      fileId,
      filename,
      mimeType,
      size: buffer.length,
      uploadedAt: new Date().toISOString(),
    }

    // Store metadata
    const metadataPath = path.join(this.metadataDir, `${fileId}.json`)
    await fs.writeFile(metadataPath, JSON.stringify(metadata, null, 2))

    logger.info('File uploaded successfully', {
      fileId,
      filename,
      size: buffer.length,
      mimeType,
    })

    return metadata
  }

  /**
   * Get file content and metadata
   */
  async getFile(fileId: string): Promise<FileWithMetadata> {
    // Read metadata first to validate file exists
    const metadata = await this.getFileMetadata(fileId)

    // Read file content
    const filePath = path.join(this.uploadDir, fileId)
    try {
      const buffer = await fs.readFile(filePath)
      return { buffer, metadata }
    } catch (error) {
      logger.error('Failed to read file', {
        fileId,
        error: error instanceof Error ? error.message : String(error),
      })
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
      logger.warn('Failed to read file metadata', {
        fileId,
        error: error instanceof Error ? error.message : String(error),
      })
      throw new Error(`File not found: ${fileId}`)
    }
  }

  /**
   * Delete a file and its metadata
   */
  async deleteFile(fileId: string): Promise<void> {
    const filePath = path.join(this.uploadDir, fileId)
    const metadataPath = path.join(this.metadataDir, `${fileId}.json`)

    try {
      // Delete file
      await fs.unlink(filePath).catch((error) => {
        logger.warn('Failed to delete file (may not exist)', {
          fileId,
          error: error instanceof Error ? error.message : String(error),
        })
      })

      // Delete metadata
      await fs.unlink(metadataPath).catch((error) => {
        logger.warn('Failed to delete metadata (may not exist)', {
          fileId,
          error: error instanceof Error ? error.message : String(error),
        })
      })

      logger.info('File deleted successfully', { fileId })
    } catch (error) {
      logger.error('Failed to delete file', {
        fileId,
        error: error instanceof Error ? error.message : String(error),
      })
      throw error
    }
  }

  /**
   * Check if a file exists
   */
  async fileExists(fileId: string): Promise<boolean> {
    const filePath = path.join(this.uploadDir, fileId)
    try {
      await fs.access(filePath)
      return true
    } catch {
      return false
    }
  }
}

export function createUploadService(): UploadService {
  return new UploadService()
}
