import { randomUUID } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { Hono } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { describeRoute } from 'hono-openapi'
import { resolver } from 'hono-openapi/zod'
import { z } from 'zod'
import type { AppBindings } from '../app.js'
import { getDataDir } from '../utils/data-dir.js'
import { getRequiredParam } from '../utils/error-handler.js'
import { logger } from '../utils/logger.js'

// Response schemas
const uploadResponseSchema = z.object({
  id: z.string(),
  filename: z.string(),
  path: z.string(),
  absolutePath: z.string(),
  size: z.number(),
  mimeType: z.string(),
  uploadedAt: z.string(),
})

export async function registerUploadRoutes(app: Hono<AppBindings>, prefix: string) {
  // Get the upload directory from env or use default
  const getUploadDir = () => {
    const dataDir = getDataDir()
    return process.env.UPLOAD_DIR || path.join(dataDir, 'uploads')
  }

  // Upload file endpoint
  app.post(
    prefix,
    describeRoute({
      description: 'Upload a file',
      requestBody: {
        description: 'File to upload',
        content: {
          'multipart/form-data': {
            schema: {
              type: 'object',
              properties: {
                file: {
                  type: 'string',
                  format: 'binary',
                  description: 'File to upload',
                },
                taskId: {
                  type: 'string',
                  description:
                    'Optional task ID - if provided, file is uploaded to task directory, otherwise to temp directory',
                },
              },
              required: ['file'],
            },
          },
        },
      },
      responses: {
        200: {
          description: 'File uploaded successfully',
          content: {
            'application/json': {
              schema: resolver(uploadResponseSchema),
            },
          },
        },
        400: {
          description: 'No file provided or invalid request',
        },
        500: {
          description: 'Failed to upload file',
        },
      },
    }),
    async (c) => {
      try {
        const formData = await c.req.formData()
        const file = formData.get('file') as File
        const taskId = formData.get('taskId') as string // Optional taskId

        if (!file) {
          throw new HTTPException(400, { message: 'No file provided' })
        }

        const buffer = await file.arrayBuffer()
        const filename = file.name
        const mimeType = file.type || 'application/octet-stream'

        // Validate filename length (255 chars is typical filesystem limit)
        if (filename.length > 255) {
          throw new HTTPException(400, { message: 'Filename too long (max 255 characters)' })
        }

        const uploadDir = getUploadDir()
        let uploadId: string
        let fileDir: string

        if (taskId) {
          // Upload directly to task directory
          uploadId = taskId
          fileDir = path.join(uploadDir, taskId)
        } else {
          // Upload to temp directory (for task creation)
          uploadId = `temp-${randomUUID()}`
          fileDir = path.join(uploadDir, 'temp', uploadId)
        }

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
    },
  )

  // Get uploaded file
  app.get(
    `${prefix}/:uploadId/:filename`,
    describeRoute({
      description: 'Download or view an uploaded file',
      parameters: [
        {
          name: 'uploadId',
          in: 'path',
          required: true,
          schema: {
            type: 'string',
          },
          description: 'Upload ID of the file',
        },
        {
          name: 'filename',
          in: 'path',
          required: true,
          schema: {
            type: 'string',
          },
          description: 'Name of the file to retrieve',
        },
      ],
      responses: {
        200: {
          description: 'File content',
          content: {
            'application/octet-stream': {
              schema: {
                type: 'string',
                format: 'binary',
              },
            },
            'image/*': {
              schema: {
                type: 'string',
                format: 'binary',
              },
            },
            'text/plain': {
              schema: {
                type: 'string',
              },
            },
          },
        },
        400: {
          description: 'Upload ID and filename are required',
        },
        404: {
          description: 'File not found',
        },
        500: {
          description: 'Failed to retrieve file',
        },
      },
    }),
    async (c) => {
      const uploadId = getRequiredParam(c, 'uploadId')
      const filename = getRequiredParam(c, 'filename')

      try {
        const uploadDir = getUploadDir()
        let filePath: string

        if (uploadId.startsWith('temp-')) {
          // Handle temp files which are stored in /uploads/temp/temp-{uuid}/filename
          filePath = path.join(uploadDir, 'temp', uploadId, filename)
        } else {
          // Handle task files which are stored in /uploads/{taskId}/filename
          filePath = path.join(uploadDir, uploadId, filename)
        }

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
    },
  )
}
