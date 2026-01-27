import {
  DeleteObjectCommand,
  GetObjectCommand,
  HeadObjectCommand,
  PutObjectCommand,
  S3Client,
} from '@aws-sdk/client-s3'
import { logger } from '../../../utils/logger'
import type { StorageBackend } from './storage-backend'

/**
 * S3StorageBackend - AWS S3 storage implementation
 *
 * Stores files in S3. Used for Kubernetes mode where files are
 * accessible via S3 CSI driver mounts in sandbox pods.
 */
export class S3StorageBackend implements StorageBackend {
  private readonly log = logger.child({ component: 'S3StorageBackend' })
  private readonly s3Client: S3Client

  constructor(
    private readonly bucketName: string,
    private readonly keyPrefix: string = 'uploads',
    region = 'us-east-1',
  ) {
    this.s3Client = new S3Client({ region })
    this.log.info({ bucketName, keyPrefix, region }, 'S3StorageBackend initialized')
  }

  private getS3Key(key: string): string {
    return `${this.keyPrefix}/${key}`
  }

  async write(key: string, buffer: Buffer): Promise<void> {
    const s3Key = this.getS3Key(key)
    const command = new PutObjectCommand({
      Bucket: this.bucketName,
      Key: s3Key,
      Body: buffer,
    })

    await this.s3Client.send(command)
    this.log.debug({ key, s3Key, size: buffer.length }, 'File written to S3')
  }

  async read(key: string): Promise<Buffer> {
    const s3Key = this.getS3Key(key)
    const command = new GetObjectCommand({
      Bucket: this.bucketName,
      Key: s3Key,
    })

    try {
      const response = await this.s3Client.send(command)
      if (!response.Body) {
        throw new Error(`Empty response body for key: ${key}`)
      }

      // Convert stream to buffer
      const chunks: Uint8Array[] = []
      const stream = response.Body as AsyncIterable<Uint8Array>
      for await (const chunk of stream) {
        chunks.push(chunk)
      }
      const buffer = Buffer.concat(chunks)

      this.log.debug({ key, s3Key, size: buffer.length }, 'File read from S3')
      return buffer
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      this.log.error({ key, s3Key, error: message }, 'Failed to read file from S3')
      throw new Error(`File not found: ${key}`)
    }
  }

  async delete(key: string): Promise<void> {
    const s3Key = this.getS3Key(key)
    const command = new DeleteObjectCommand({
      Bucket: this.bucketName,
      Key: s3Key,
    })

    try {
      await this.s3Client.send(command)
      this.log.debug({ key, s3Key }, 'File deleted from S3')
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      this.log.warn({ key, s3Key, error: message }, 'Failed to delete file from S3')
    }
  }

  async exists(key: string): Promise<boolean> {
    const s3Key = this.getS3Key(key)
    const command = new HeadObjectCommand({
      Bucket: this.bucketName,
      Key: s3Key,
    })

    try {
      await this.s3Client.send(command)
      return true
    } catch {
      return false
    }
  }
}
