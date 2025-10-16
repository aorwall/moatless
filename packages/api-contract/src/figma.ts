import { createRoute } from '@hono/zod-openapi'
import {
  FigmaClipboardImportRequestSchema,
  FigmaImportIdParamsSchema,
  FigmaImportMetadataSchema,
  FigmaImportRequestSchema,
  FigmaImportResponseSchema,
} from '@moatless/types'

// POST /api/v1/figma/import - Import a Figma node
export const importFigmaNodeRoute = createRoute({
  method: 'post',
  path: '/api/v1/figma/import',
  tags: ['Figma'],
  description:
    'Import a Figma node by URL. Fetches node metadata and rendered image, stores them with a unique import ID.',
  request: {
    body: {
      content: {
        'application/json': {
          schema: FigmaImportRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Node imported successfully',
      content: {
        'application/json': {
          schema: FigmaImportResponseSchema,
        },
      },
    },
    400: { description: 'Invalid Figma URL or missing node-id parameter' },
    500: { description: 'Failed to import node (Figma API error or missing FIGMA_ACCESS_TOKEN)' },
  },
})

// POST /api/v1/figma/import-clipboard - Import Figma clipboard data
export const importFigmaClipboardRoute = createRoute({
  method: 'post',
  path: '/api/v1/figma/import-clipboard',
  tags: ['Figma'],
  description:
    'Import Figma design from clipboard data. Parses the fig-kiwie binary format, stores metadata, and returns import details. No API token required.',
  request: {
    body: {
      content: {
        'application/json': {
          schema: FigmaClipboardImportRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Clipboard data imported successfully',
      content: {
        'application/json': {
          schema: FigmaImportResponseSchema,
        },
      },
    },
    400: { description: 'Invalid clipboard format or missing data' },
    500: { description: 'Failed to parse clipboard data' },
  },
})

// GET /api/v1/figma/imports/{importId} - Get Figma import metadata
export const getFigmaImportRoute = createRoute({
  method: 'get',
  path: '/api/v1/figma/imports/{importId}',
  tags: ['Figma'],
  description: 'Retrieve metadata for a Figma import, including the simplified node structure',
  request: {
    params: FigmaImportIdParamsSchema,
  },
  responses: {
    200: {
      description: 'Import metadata retrieved successfully',
      content: {
        'application/json': {
          schema: FigmaImportMetadataSchema,
        },
      },
    },
    404: { description: 'Import not found' },
    500: { description: 'Internal server error' },
  },
})

// GET /api/v1/figma/imports/{importId}/image - Get Figma import image
export const getFigmaImportImageRoute = createRoute({
  method: 'get',
  path: '/api/v1/figma/imports/{importId}/image',
  tags: ['Figma'],
  description: 'Retrieve the rendered image for a Figma import',
  request: {
    params: FigmaImportIdParamsSchema,
  },
  responses: {
    200: {
      description: 'Image retrieved successfully (PNG format)',
      content: {
        'image/png': {
          schema: {
            type: 'string',
            format: 'binary',
          },
        },
      },
    },
    404: { description: 'Import or image file not found' },
    500: { description: 'Internal server error' },
  },
})
