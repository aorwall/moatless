import { createRoute } from '@hono/zod-openapi'
import { z } from 'zod'

/**
 * API Key response schema (without the hash, for listing)
 */
export const apiKeySchema = z.object({
  id: z.string(),
  name: z.string(),
  keyPrefix: z.string(),
  scopes: z.array(z.string()).nullable(),
  lastUsedAt: z.string().nullable(),
  expiresAt: z.string().nullable(),
  createdAt: z.string(),
})

/**
 * API Key creation result schema (includes the full key, shown only once)
 */
export const apiKeyCreateResultSchema = z.object({
  id: z.string(),
  key: z.string(),
  name: z.string(),
  keyPrefix: z.string(),
  scopes: z.array(z.string()).nullable(),
  expiresAt: z.string().nullable(),
  createdAt: z.string(),
})

/**
 * API Key creation request schema
 */
export const createApiKeyRequestSchema = z.object({
  name: z.string().min(1).max(255),
  scopes: z.array(z.string()).optional(),
  expiresInDays: z.number().int().positive().optional(),
})

/**
 * POST /api/v1/auth/api-keys
 * Create a new API key
 */
export const createApiKeyRoute = createRoute({
  method: 'post',
  path: '/api/v1/auth/api-keys',
  tags: ['API Keys'],
  description: 'Create a new API key for programmatic access. The full key is only returned once!',
  request: {
    body: {
      content: {
        'application/json': {
          schema: createApiKeyRequestSchema,
        },
      },
    },
  },
  responses: {
    201: {
      description: 'API key created successfully',
      content: {
        'application/json': {
          schema: apiKeyCreateResultSchema,
        },
      },
    },
    401: {
      description: 'Unauthorized - requires valid session',
    },
  },
})

/**
 * GET /api/v1/auth/api-keys
 * List all API keys for the current user
 */
export const listApiKeysRoute = createRoute({
  method: 'get',
  path: '/api/v1/auth/api-keys',
  tags: ['API Keys'],
  description: 'List all API keys for the current user. Does not include the actual keys.',
  responses: {
    200: {
      description: 'List of API keys',
      content: {
        'application/json': {
          schema: z.array(apiKeySchema),
        },
      },
    },
    401: {
      description: 'Unauthorized - requires valid session',
    },
  },
})

/**
 * DELETE /api/v1/auth/api-keys/:keyId
 * Revoke an API key
 */
export const revokeApiKeyRoute = createRoute({
  method: 'delete',
  path: '/api/v1/auth/api-keys/{keyId}',
  tags: ['API Keys'],
  description: 'Revoke an API key. The key will no longer be valid for authentication.',
  request: {
    params: z.object({
      keyId: z.string(),
    }),
  },
  responses: {
    200: {
      description: 'API key revoked successfully',
      content: {
        'application/json': {
          schema: z.object({ message: z.string() }),
        },
      },
    },
    401: {
      description: 'Unauthorized - requires valid session',
    },
    404: {
      description: 'API key not found or not owned by user',
    },
  },
})
