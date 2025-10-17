import { createRoute } from '@hono/zod-openapi'
import {
  FigmaClipboardImportRequestSchema,
  FigmaImportIdParamsSchema,
  FigmaImportMetadataSchema,
  FigmaImportRequestSchema,
  FigmaImportResponseSchema,
  FigmaOAuthTokenRequestSchema,
  FigmaOAuthUrlResponseSchema,
  FigmaPATRequestSchema,
} from '@moatless/types'
import { z } from 'zod'

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

// GET /api/v1/figma/oauth/url - Get Figma OAuth authorization URL
export const getFigmaOAuthUrlRoute = createRoute({
  method: 'get',
  path: '/api/v1/figma/oauth/url',
  tags: ['Figma'],
  description:
    'Generate Figma OAuth authorization URL with PKCE for the current user. Requires authentication.',
  responses: {
    200: {
      description: 'OAuth URL generated successfully',
      content: {
        'application/json': {
          schema: FigmaOAuthUrlResponseSchema,
        },
      },
    },
    401: { description: 'Not authenticated' },
    500: { description: 'Failed to generate OAuth URL or missing Figma OAuth configuration' },
  },
})

// POST /api/v1/figma/oauth/token - Exchange OAuth code for access token
export const exchangeFigmaTokenRoute = createRoute({
  method: 'post',
  path: '/api/v1/figma/oauth/token',
  tags: ['Figma'],
  description:
    'Exchange Figma OAuth authorization code for access token. Validates PKCE and stores token for authenticated user.',
  request: {
    body: {
      content: {
        'application/json': {
          schema: FigmaOAuthTokenRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Token exchanged and saved successfully',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            message: z.string(),
          }),
        },
      },
    },
    400: { description: 'Invalid code or state parameter' },
    401: { description: 'Not authenticated' },
    500: { description: 'Failed to exchange token with Figma API' },
  },
})

// GET /api/v1/figma/config - Get user's Figma configuration
export const getFigmaConfigRoute = createRoute({
  method: 'get',
  path: '/api/v1/figma/config',
  tags: ['Figma'],
  description: "Get current user's Figma configuration (OAuth or PAT, without sensitive tokens)",
  responses: {
    200: {
      description: 'Configuration retrieved successfully',
      content: {
        'application/json': {
          schema: z.object({
            connected: z.boolean(),
            authMethod: z.enum(['oauth', 'pat']).optional(),
            figmaUserId: z.string().optional(),
            connectedAt: z.string().optional(),
            expiresAt: z.string().optional(),
          }),
        },
      },
    },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

// DELETE /api/v1/figma/config - Disconnect Figma OAuth
export const deleteFigmaConfigRoute = createRoute({
  method: 'delete',
  path: '/api/v1/figma/config',
  tags: ['Figma'],
  description: 'Disconnect Figma OAuth for the current user by deleting stored tokens',
  responses: {
    200: {
      description: 'Figma connection removed successfully',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            message: z.string(),
          }),
        },
      },
    },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})

// PUT /api/v1/figma/config/token - Save Personal Access Token
export const saveFigmaPATRoute = createRoute({
  method: 'put',
  path: '/api/v1/figma/config/token',
  tags: ['Figma'],
  description: 'Save a Figma Personal Access Token for the current user',
  request: {
    body: {
      content: {
        'application/json': {
          schema: FigmaPATRequestSchema,
        },
      },
    },
  },
  responses: {
    200: {
      description: 'Personal Access Token saved successfully',
      content: {
        'application/json': {
          schema: z.object({
            success: z.boolean(),
            message: z.string(),
          }),
        },
      },
    },
    400: { description: 'Invalid token' },
    401: { description: 'Not authenticated' },
    500: { description: 'Internal server error' },
  },
})
