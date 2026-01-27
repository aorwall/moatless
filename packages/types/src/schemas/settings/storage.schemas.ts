import { z } from 'zod'

/**
 * File storage backend type
 * - local: Local filesystem storage (Docker mode)
 * - s3: AWS S3 storage (Kubernetes mode with CSI driver)
 */
export const FileStorageBackendSchema = z.enum(['local', 's3'])

/**
 * Storage settings for file uploads
 *
 * In Docker mode: Files stored locally, mounted to sandbox containers via volume
 * In K8s mode: Files stored in S3, mounted to sandbox pods via S3 CSI driver
 *
 * Environment variables:
 * - FILE_STORAGE_BACKEND: 'local' or 's3' (default: 'local')
 * - S3_BUCKET_NAME: S3 bucket for uploads
 * - S3_REGION: AWS region (default: 'us-east-1')
 * - S3_KEY_PREFIX: Key prefix in bucket (default: 'uploads')
 * - SANDBOX_UPLOAD_MOUNT_PATH: Path where uploads are mounted in sandbox (default: '/mnt/uploads')
 */
export const StorageSettingsSchema = z.object({
  /**
   * Storage backend type
   * @default 'local'
   */
  backend: FileStorageBackendSchema.default('local'),

  /**
   * S3 bucket name for uploads (required when backend is 's3')
   */
  s3BucketName: z.string().optional(),

  /**
   * AWS region for S3
   * @default 'us-east-1'
   */
  s3Region: z.string().default('us-east-1'),

  /**
   * Key prefix in S3 bucket
   * @default 'uploads'
   */
  s3KeyPrefix: z.string().default('uploads'),

  /**
   * Mount path for uploads in sandbox containers
   * Both Docker and K8s modes mount files here for consistent agent access
   * @default '/mnt/uploads'
   */
  sandboxMountPath: z.string().default('/mnt/uploads'),
})

export type FileStorageBackend = z.infer<typeof FileStorageBackendSchema>
export type StorageSettings = z.infer<typeof StorageSettingsSchema>
