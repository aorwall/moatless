import { access } from 'node:fs/promises'
import type { Hono } from 'hono'
import type { AppBindings } from '../../../app'

/**
 * Register workspace-related routes
 * Used for K8s readiness probes and workspace status
 */
export function registerWorkspaceRoutes(app: Hono<AppBindings>) {
  // GET /workspace/status - Used as K8s readiness probe
  app.get('/workspace/status', async (c) => {
    const config = c.get('config')

    // Workspace is provisioned by the entrypoint script
    // Just verify the directory exists
    let workspaceExists = false
    try {
      await access(config.workspaceDir)
      workspaceExists = true
    } catch {
      // Directory doesn't exist yet
    }

    return c.json({
      taskId: config.taskId,
      workspaceDir: config.workspaceDir,
      status: workspaceExists ? 'ready' : 'pending',
    })
  })
}
