import { DefaultAzureCredential } from '@azure/identity'
import { BlobServiceClient } from '@azure/storage-blob'
import { logger } from '../../../utils/logger'
import type { StorageBackend } from './storage-backend'

/**
 * AzureBlobStorageBackend - Azure Blob Storage implementation
 *
 * Stores files in Azure Blob Storage. Used for AKS mode where files are
 * accessible via Blob CSI driver mounts in sandbox pods.
 *
 * Authentication uses DefaultAzureCredential which supports:
 * - Workload Identity (recommended for AKS)
 * - Managed Identity
 * - Environment variables (AZURE_CLIENT_ID, AZURE_TENANT_ID, AZURE_CLIENT_SECRET)
 * - Azure CLI (local development)
 */
export class AzureBlobStorageBackend implements StorageBackend {
  private readonly log = logger.child({ component: 'AzureBlobStorageBackend' })
  private readonly blobServiceClient: BlobServiceClient
  private readonly containerName: string
  private readonly keyPrefix: string

  constructor(accountName: string, containerName: string, keyPrefix = 'uploads') {
    this.containerName = containerName
    this.keyPrefix = keyPrefix

    const credential = new DefaultAzureCredential()
    this.blobServiceClient = new BlobServiceClient(
      `https://${accountName}.blob.core.windows.net`,
      credential,
    )

    this.log.info({ accountName, containerName, keyPrefix }, 'AzureBlobStorageBackend initialized')
  }

  private getBlobName(key: string): string {
    return `${this.keyPrefix}/${key}`
  }

  private get containerClient() {
    return this.blobServiceClient.getContainerClient(this.containerName)
  }

  async write(key: string, buffer: Buffer): Promise<void> {
    const blobName = this.getBlobName(key)
    const blockBlobClient = this.containerClient.getBlockBlobClient(blobName)

    await blockBlobClient.uploadData(buffer)
    this.log.debug({ key, blobName, size: buffer.length }, 'File written to Azure Blob')
  }

  async read(key: string): Promise<Buffer> {
    const blobName = this.getBlobName(key)
    const blockBlobClient = this.containerClient.getBlockBlobClient(blobName)

    try {
      const buffer = await blockBlobClient.downloadToBuffer()
      this.log.debug({ key, blobName, size: buffer.length }, 'File read from Azure Blob')
      return buffer
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      this.log.error({ key, blobName, error: message }, 'Failed to read file from Azure Blob')
      throw new Error(`File not found: ${key}`)
    }
  }

  async delete(key: string): Promise<void> {
    const blobName = this.getBlobName(key)
    const blockBlobClient = this.containerClient.getBlockBlobClient(blobName)

    try {
      await blockBlobClient.deleteIfExists()
      this.log.debug({ key, blobName }, 'File deleted from Azure Blob')
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      this.log.warn({ key, blobName, error: message }, 'Failed to delete file from Azure Blob')
    }
  }

  async exists(key: string): Promise<boolean> {
    const blobName = this.getBlobName(key)
    const blockBlobClient = this.containerClient.getBlockBlobClient(blobName)

    try {
      await blockBlobClient.getProperties()
      return true
    } catch {
      return false
    }
  }

  async list(prefix: string): Promise<string[]> {
    const blobPrefix = this.getBlobName(prefix)
    const keys: string[] = []

    for await (const blob of this.containerClient.listBlobsFlat({ prefix: blobPrefix })) {
      const stripped = blob.name.replace(`${this.keyPrefix}/`, '')
      if (stripped) {
        keys.push(stripped)
      }
    }

    return keys
  }
}
