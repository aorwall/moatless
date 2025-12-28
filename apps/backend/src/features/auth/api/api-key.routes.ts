import type { OpenAPIHono } from '@hono/zod-openapi'
import {
  createApiKeyRoute,
  listApiKeysRoute,
  revokeApiKeyRoute,
} from '@moatless/api-contract/api-keys'
import { HTTPException } from 'hono/http-exception'
import type { AppBindings } from '../../../app'
import { appContainer } from '../../../container/container'
import { getSessionFromContext } from '../../../middleware/auth.middleware'
import { ApiKeyService } from '../services/api-key.service'

/**
 * Register API key management routes
 */
export function registerApiKeyRoutes(app: OpenAPIHono<AppBindings>) {
  /**
   * POST /api/v1/auth/api-keys
   * Create a new API key
   */
  app.openapi(createApiKeyRoute, async (c) => {
    const session = getSessionFromContext(c)
    if (!session) {
      throw new HTTPException(401, { message: 'Authentication required' })
    }

    const { name, scopes, expiresInDays } = c.req.valid('json')

    const expiresAt = expiresInDays
      ? new Date(Date.now() + expiresInDays * 24 * 60 * 60 * 1000)
      : undefined

    const apiKeyService = appContainer.get(ApiKeyService)
    const result = await apiKeyService.generateKey(
      session.userId,
      name,
      scopes,
      expiresAt,
      session.groupId,
    )

    return c.json(
      {
        id: result.id,
        key: result.key,
        name: result.name,
        keyPrefix: result.keyPrefix,
        scopes: result.scopes,
        expiresAt: result.expiresAt?.toISOString() ?? null,
        createdAt: result.createdAt.toISOString(),
      },
      201,
    )
  })

  /**
   * GET /api/v1/auth/api-keys
   * List all API keys for the current user
   */
  app.openapi(listApiKeysRoute, async (c) => {
    const session = getSessionFromContext(c)
    if (!session) {
      throw new HTTPException(401, { message: 'Authentication required' })
    }

    const apiKeyService = appContainer.get(ApiKeyService)
    const keys = await apiKeyService.listKeys(session.userId)

    return c.json(
      keys.map((key) => ({
        id: key.id,
        name: key.name,
        keyPrefix: key.keyPrefix,
        scopes: key.scopes,
        lastUsedAt: key.lastUsedAt?.toISOString() ?? null,
        expiresAt: key.expiresAt?.toISOString() ?? null,
        createdAt: key.createdAt.toISOString(),
      })),
    )
  })

  /**
   * DELETE /api/v1/auth/api-keys/:keyId
   * Revoke an API key
   */
  app.openapi(revokeApiKeyRoute, async (c) => {
    const session = getSessionFromContext(c)
    if (!session) {
      throw new HTTPException(401, { message: 'Authentication required' })
    }

    const { keyId } = c.req.valid('param')

    const apiKeyService = appContainer.get(ApiKeyService)
    const revoked = await apiKeyService.revokeKey(keyId, session.userId)

    if (!revoked) {
      throw new HTTPException(404, { message: 'API key not found or not owned by user' })
    }

    return c.json({ message: 'API key revoked successfully' })
  })
}
