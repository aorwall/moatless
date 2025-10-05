import { createRoute } from '@hono/zod-openapi'
import { UploadFileResponseSchema } from '@moatless/types'
import { pathParams } from '../params.js'

// POST /api/v1/uploads - Upload file
export const uploadFileRoute = createRoute({
  method: 'post',
  path: '/api/v1/uploads',
  tags: ['Uploads'],
  description:
    'Upload a file to the server. File can be associated with a task or stored temporarily.',
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
              taskId: {
                type: 'string',
                description: 'Optional task ID to associate the file with',
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

// GET /api/v1/uploads/{uploadId}/{filename} - Retrieve uploaded file
export const getUploadedFileRoute = createRoute({
  method: 'get',
  path: '/api/v1/uploads/{uploadId}/{filename}',
  tags: ['Uploads'],
  description: 'Retrieve an uploaded file by its upload ID and filename',
  request: {
    params: pathParams.uploadIdAndFilename,
  },
  responses: {
    200: {
      description: 'File retrieved successfully (various MIME types supported)',
    },
    404: { description: 'File not found' },
    500: { description: 'Internal server error' },
  },
})
