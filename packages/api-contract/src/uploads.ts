import { createRoute } from '@hono/zod-openapi'
import { UploadFileIdParamsSchema, UploadFileResponseSchema } from '@moatless/types'

// POST /api/v1/uploads - Upload file
export const uploadFileRoute = createRoute({
  method: 'post',
  path: '/api/v1/uploads',
  tags: ['Uploads'],
  description: 'Upload a file to the server and receive a unique fileId.',
  request: {
    body: {
      content: {
        'multipart/form-data': {
          schema: {
            type: 'object',
            properties: {
              file: {
                type: 'string',
                format: 'binary',
              },
            },
            required: ['file'],
          },
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Success',
      content: {
        'application/json': {
          schema: UploadFileResponseSchema,
        },
      },
    },
  },
})

// GET /api/v1/uploads/{fileId} - Retrieve uploaded file
export const getUploadedFileRoute = createRoute({
  method: 'get',
  path: '/api/v1/uploads/{fileId}',
  tags: ['Uploads'],
  description: 'Retrieve an uploaded file by its fileId',
  request: {
    params: UploadFileIdParamsSchema,
  },
  responses: {
    200: {
      description: 'File retrieved successfully (various MIME types supported)',
    },
    404: { description: 'File not found' },
    500: { description: 'Internal server error' },
  },
})
