import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../app.js'
import { logger } from '../utils/logger.js'

export async function registerUploadRoutes(app: Hono<AppBindings>, prefix: string) {
  // Get the upload directory from env or use default
  const getUploadDir = () => {
    return process.env.UPLOAD_DIR || path.join(process.cwd(), 'uploads')
  }

  // Upload file endpoint
  app.post(prefix, async (c) => {
    try {
      const formData = await c.req.formData()
      const file = formData.get('file') as File
      
      if (!file) {
        throw new HTTPException(400, { message: 'No file provided' })
      }

      const buffer = await file.arrayBuffer()
      const filename = file.name
      const mimeType = file.type || 'application/octet-stream'

      // Generate unique directory for this upload
      const uploadId = randomUUID()
      const uploadDir = getUploadDir()
      const fileDir = path.join(uploadDir, uploadId)

      // Create directory
      await fs.mkdir(fileDir, { recursive: true })

      // Sanitize filename
      const safeName = path.basename(filename)
      const filePath = path.join(fileDir, safeName)

      // Save file
      await fs.writeFile(filePath, new Uint8Array(buffer))

      // Return file info
      const relativePath = `/uploads/${uploadId}/${safeName}`
      const absolutePath = filePath

      logger.info('File uploaded successfully', {
        uploadId,
        filename: safeName,
        size: buffer.byteLength,
        path: relativePath,
      })

      return c.json({
        id: uploadId,
        filename: safeName,
        path: relativePath,
        absolutePath,
        size: buffer.byteLength,
        mimeType,
        uploadedAt: new Date().toISOString(),
      })
    } catch (error) {
      console.error('Failed to upload file', error)
      if (error instanceof HTTPException) {
        throw error
      }
      if (error instanceof Error) {
        throw new HTTPException(500, { message: error.message })
      }
      throw new HTTPException(500, { message: 'Failed to upload file' })
    }
  })

  // Get uploaded file
  app.get(`${prefix}/:uploadId/:filename`, async (c) => {
    const uploadId = c.req.param('uploadId')
    const filename = c.req.param('filename')
    
    if (!uploadId || !filename) {
      throw new HTTPException(400, { message: 'Upload ID and filename are required' })
    }

    try {
      const uploadDir = getUploadDir()
      const filePath = path.join(uploadDir, uploadId, filename)

      // Check if file exists
      try {
        await fs.access(filePath)
      } catch {
        throw new HTTPException(404, { message: 'File not found' })
      }

      // Read file
      const buffer = await fs.readFile(filePath)

      // Determine content type from extension
      const ext = path.extname(filename).toLowerCase()
      const mimeTypes: Record<string, string> = {
        '.jpg': 'image/jpeg',
        '.jpeg': 'image/jpeg',
        '.png': 'image/png',
        '.gif': 'image/gif',
        '.webp': 'image/webp',
        '.pdf': 'application/pdf',
        '.txt': 'text/plain',
        '.md': 'text/markdown',
        '.json': 'application/json',
        '.js': 'application/javascript',
        '.ts': 'application/typescript',
        '.html': 'text/html',
        '.css': 'text/css',
        '.xml': 'application/xml',
      }

      const mimeType = mimeTypes[ext] || 'application/octet-stream'

      // Set cache headers for images
      const headers: Record<string, string> = {
        'Content-Type': mimeType,
      }
      if (mimeType.startsWith('image/')) {
        headers['Cache-Control'] = 'public, max-age=31536000'
      }

      return new Response(buffer, {
        headers,
      })
    } catch (error) {
      console.error('Failed to get file', error)
      if (error instanceof HTTPException) {
        throw error
      }
      throw new HTTPException(500, { message: 'Failed to retrieve file' })
    }
  })
}
