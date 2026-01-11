import { serve } from '@hono/node-server'
import { createApp } from './app'
import { loadConfig } from './config/environment'
import { provisionWorkspace } from './features/workspace'
import { logger } from './shared/logger'

/**
 * Sandbox application entry point
 *
 * This is a standalone HTTP server that provides execution environment
 * capabilities for the Moatless system. It handles:
 * - Agent execution (ClaudeCodeAgent)
 * - File operations (read, write, list, search)
 * - Git operations (status, diff, changes)
 * - Preview server lifecycle
 * - Workspace provisioning (at startup)
 *
 * Communication with the main server is via HTTP API and PostgreSQL events.
 */
async function main() {
  try {
    const config = loadConfig()

    logger.info(
      {
        taskId: config.taskId,
        workspaceDir: config.workspaceDir,
        dataDir: config.dataDir,
        port: config.port,
        gitCloneUrl: config.gitCloneUrl,
        gitBranch: config.gitBranch,
      },
      'Starting sandbox application',
    )

    // Auto-provision workspace at startup
    const provisioningResult = await provisionWorkspace(config)

    if (provisioningResult.status === 'failed') {
      logger.error(
        { error: provisioningResult.error },
        'Workspace provisioning failed, starting server anyway for status reporting',
      )
    }

    const app = createApp(config, provisioningResult)

    // Start the HTTP server
    serve(
      {
        fetch: app.fetch,
        port: config.port,
      },
      (info) => {
        logger.info(
          {
            port: info.port,
            taskId: config.taskId,
            provisioningStatus: provisioningResult.status,
          },
          `Sandbox HTTP server listening on port ${info.port}`,
        )
      },
    )
  } catch (error) {
    logger.error({ err: error }, 'Failed to start sandbox application')
    process.exit(1)
  }
}

main()
