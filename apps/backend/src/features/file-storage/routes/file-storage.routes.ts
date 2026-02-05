import type { OpenAPIHono } from '@hono/zod-openapi'
import { getUploadedFileRoute, uploadFileRoute } from '@moatless/api-contract/uploads'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { logger } from '../../../utils/logger'
import { FileStorageService } from '../services/file-storage.service'

export function registerFileStorageRoutes(app: OpenAPIHono<AppBindings>) {
  /**
   * POST /api/v1/uploads
   * Upload a file to the server
   */
  app.openapi(uploadFileRoute, async (c) => {
    try {
      const fileStorageService = appContainer.get(FileStorageService)
      const formData = await c.req.formData()
      const file = formData.get('file') as File

      if (!file) {
        throw new HTTPException(400, { message: 'No file provided' })
      }

      const buffer = await file.arrayBuffer()
      const filename = file.name
      const mimeType = file.type || 'application/octet-stream'

      // Use file storage service to store file
      const uploadedFile = await fileStorageService.uploadFile(
        Buffer.from(buffer),
        filename,
        mimeType,
      )

      return c.json(uploadedFile)
    } catch (error) {
      logger.error({ err: error }, 'Failed to upload file')
      if (error instanceof HTTPException) {
        throw error
      }
      if (error instanceof Error) {
        throw new HTTPException(500, { message: error.message })
      }
      throw new HTTPException(500, { message: 'Failed to upload file' })
    }
  })

  /**
   * GET /api/v1/uploads/{fileId}
   * Retrieve an uploaded file by its fileId
   */
  app.openapi(getUploadedFileRoute, async (c) => {
    const params = c.req.valid('param')
    const fileId = params.fileId as string

    try {
      const fileStorageService = appContainer.get(FileStorageService)

      // Get file and metadata from file storage service
      const { buffer, metadata } = await fileStorageService.getFile(fileId)

      // Set headers including Content-Disposition for filename
      const headers: Record<string, string> = {
        'Content-Type': metadata.mimeType,
        'Content-Disposition': `attachment; filename="${metadata.filename}"`,
      }
      if (metadata.mimeType.startsWith('image/')) {
        headers['Cache-Control'] = 'public, max-age=31536000'
      }

      return new Response(buffer, { headers })
    } catch (error) {
      logger.error({ err: error }, 'Failed to get file')
      if (error instanceof Error && error.message.includes('not found')) {
        throw new HTTPException(404, { message: 'File not found' })
      }
      throw new HTTPException(500, { message: 'Failed to retrieve file' })
    }
  })
}
