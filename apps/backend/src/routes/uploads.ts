import type { OpenAPIHono } from '@hono/zod-openapi'
import { getUploadedFileRoute, uploadFileRoute } from '@moatless/api-contract/uploads'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'
import { appContainer } from '../container/container.js'
import { UploadService } from '../services/upload.service.js'
import { logger } from '../utils/logger.js'

export function registerUploadRoutes(app: OpenAPIHono<AppBindings>) {
  /**
   * POST /api/v1/uploads
   * Upload a file to the server
   */
  app.openapi(uploadFileRoute, async (c) => {
    try {
      const uploadService = appContainer.get(UploadService)
      const formData = await c.req.formData()
      const file = formData.get('file') as File

      if (!file) {
        throw new HTTPException(400, { message: 'No file provided' })
      }

      const buffer = await file.arrayBuffer()
      const filename = file.name
      const mimeType = file.type || 'application/octet-stream'

      // Use upload service to store file
      const uploadedFile = await uploadService.uploadFile(Buffer.from(buffer), filename, mimeType)

      return c.json(uploadedFile)
    } catch (error) {
      logger.error('Failed to upload file', error)
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
      const uploadService = appContainer.get(UploadService)

      // Get file and metadata from upload service
      const { buffer, metadata } = await uploadService.getFile(fileId)

      // Set cache headers for images
      const headers: Record<string, string> = {
        'Content-Type': metadata.mimeType,
      }
      if (metadata.mimeType.startsWith('image/')) {
        headers['Cache-Control'] = 'public, max-age=31536000'
      }

      return new Response(buffer, { headers })
    } catch (error) {
      logger.error('Failed to get file', error)
      if (error instanceof Error && error.message.includes('not found')) {
        throw new HTTPException(404, { message: 'File not found' })
      }
      throw new HTTPException(500, { message: 'Failed to retrieve file' })
    }
  })
}
