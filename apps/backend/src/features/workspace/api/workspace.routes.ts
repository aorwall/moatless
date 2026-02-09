import type { OpenAPIHono } from '@hono/zod-openapi'
import type { AppBindings } from '../../../app'

/**
 * Register workspace management routes.
 *
 * Workspace status, retry, and restart are now handled directly by the
 * Rust backend (frontend calls it via sandboxApi). This function
 * is kept as a no-op so the route registration call site doesn't break.
 */
export function registerWorkspaceRoutes(_app: OpenAPIHono<AppBindings>) {
  // All routes moved to Rust backend — nothing to register
}
