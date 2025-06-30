import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { FastifyInstance } from 'fastify'
import { z } from 'zod'
import { UploadFileResponseSchema } from '../../shared/types/api.js'
import { logger } from '../utils/logger.js'

export async function uploadRoutes(app: FastifyInstance) {
  // Get the upload directory from env or use default
  const getUploadDir = () => {
    return process.env.UPLOAD_DIR || path.join(process.cwd(), 'uploads')
  }

  // Upload file endpoint
  app.post(
    '/',
    {
      schema: {
        response: {
          200: UploadFileResponseSchema,
        },
      },
    },
    async (request, reply) => {
      const data = await request.file()
      if (!data) {
        return reply.status(400).send({ error: 'No file provided' })
      }

      try {
        const buffer = await data.toBuffer()
        const filename = data.filename
        const mimeType = data.mimetype || 'application/octet-stream'

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
        await fs.writeFile(filePath, buffer)

        // Return file info
        const relativePath = `/uploads/${uploadId}/${safeName}`
        const absolutePath = filePath

        logger.info('File uploaded successfully', {
          uploadId,
          filename: safeName,
          size: buffer.length,
          path: relativePath,
        })

        return {
          id: uploadId,
          filename: safeName,
          path: relativePath,
          absolutePath,
          size: buffer.length,
          mimeType,
          uploadedAt: new Date().toISOString(),
        }
      } catch (error) {
        request.log.error('Failed to upload file', error as Error)
        if (error instanceof Error) {
          return reply.status(500).send({ error: error.message })
        }
        return reply.status(500).send({ error: 'Failed to upload file' })
      }
    },
  )

  // Get uploaded file
  app.get(
    '/:uploadId/:filename',
    {
      schema: {
        params: z.object({
          uploadId: z.string(),
          filename: z.string(),
        }),
      },
    },
    async (request, reply) => {
      const { uploadId, filename } = request.params as { uploadId: string; filename: string }

      try {
        const uploadDir = getUploadDir()
        const filePath = path.join(uploadDir, uploadId, filename)

        // Check if file exists
        try {
          await fs.access(filePath)
        } catch {
          return reply.status(404).send({ error: 'File not found' })
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
        reply.type(mimeType)

        // Set cache headers for images
        if (mimeType.startsWith('image/')) {
          reply.header('Cache-Control', 'public, max-age=31536000')
        }

        return reply.send(buffer)
      } catch (error) {
        request.log.error('Failed to get file', error as Error)
        return reply.status(500).send({ error: 'Failed to retrieve file' })
      }
    },
  )
}
