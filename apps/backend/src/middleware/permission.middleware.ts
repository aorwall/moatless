import type { Action, Resource } from '@moatless/types'
import type { Context, Next } from 'hono'
import { HTTPException } from 'hono/http-exception'
import { appContainer } from '../container/container'
import { PermissionService } from '../features/auth/services/permission.service'

/**
 * Create middleware that requires specific permission.
 *
 * @example
 * // Protect a route requiring admin access
 * app.use('/api/v1/admin/*', requirePermission('manage', 'system'))
 *
 * // Protect settings write operations
 * app.put('/api/v1/settings/*', requirePermission('update', 'settings'))
 */
export function requirePermission(action: Action, resource: Resource) {
  return (_c: Context, next: Next) => {
    const permissionService = appContainer.get(PermissionService)

    const allowed = permissionService.can(action, resource)
    if (!allowed) {
      throw new HTTPException(403, {
        message: `Permission denied: cannot ${action} ${resource}`,
      })
    }

    return next()
  }
}

/**
 * Middleware that requires admin role.
 * Shorthand for requirePermission('manage', 'system')
 */
export const requireAdmin = (_c: Context, next: Next) => {
  const permissionService = appContainer.get(PermissionService)

  if (!permissionService.isAdmin()) {
    throw new HTTPException(403, {
      message: 'Admin access required',
    })
  }

  return next()
}
