import type { Hono } from 'hono'
import type { AppBindings } from '../../../app'
import { logger } from '../../../shared/logger'
import { GitService } from '../services/git.service'

/**
 * Register git operation routes
 */
export function registerGitRoutes(app: Hono<AppBindings>) {
  // GET /git-status - Get git repository status
  app.get('/git-status', async (c) => {
    const { config } = c.var
    const gitService = new GitService(config)

    try {
      const status = await gitService.getStatus()
      return c.json(status)
    } catch (error) {
      logger.error({ err: error as Error, taskId: config.taskId }, 'Failed to get git status')
      return c.json({ error: error instanceof Error ? error.message : String(error) }, 500)
    }
  })

  // GET /git-changes - Get working directory changes
  app.get('/git-changes', async (c) => {
    const { config } = c.var
    const gitService = new GitService(config)

    try {
      const changes = await gitService.getChanges()
      return c.json(changes)
    } catch (error) {
      logger.error({ err: error as Error, taskId: config.taskId }, 'Failed to get git changes')
      return c.json({ error: error instanceof Error ? error.message : String(error) }, 500)
    }
  })

  // GET /git-diff - Get detailed git diff
  app.get('/git-diff', async (c) => {
    const { config } = c.var
    const gitService = new GitService(config)

    try {
      const diff = await gitService.getDiff()
      return c.json(diff)
    } catch (error) {
      logger.error({ err: error as Error, taskId: config.taskId }, 'Failed to get git diff')
      return c.json(
        { files: [], error: error instanceof Error ? error.message : String(error) },
        500,
      )
    }
  })

  // GET /current-commit-hash - Get current commit hash
  app.get('/current-commit-hash', async (c) => {
    const { config } = c.var
    const gitService = new GitService(config)

    try {
      const hash = await gitService.getCurrentCommitHash()
      return c.json({ hash })
    } catch (error) {
      logger.error({ err: error as Error, taskId: config.taskId }, 'Failed to get commit hash')
      return c.json({ hash: null })
    }
  })
}
