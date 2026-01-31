import { serve } from '@hono/node-server'
import { createApp } from './app'
import { loadConfig } from './config/environment'
import { createAgentRequestConsumer } from './features/agents'
import { createSandboxEventService } from './features/events/services/sandbox-event.service'
import { initializeMonitor } from './features/preview/services/preview-process-monitor'
import { logger } from './shared/logger'

/**
 * Sandbox application entry point
 *
 * This is a standalone HTTP server that provides execution environment
 * capabilities for the Moatless system. It handles:
 * - Agent execution (ClaudeCodeAgent)
 * - File operations (read, write, list, search)
 * - Git operations (status, diff, changes)
 * - Preview server monitoring (via filesystem artifacts)
 *
 * The preview server process is managed externally by the entrypoint script
 * and the Claude Code agent via /opt/moatless/bin/preview-server.
 * This app monitors its state via filesystem (PID, status, log files).
 *
 * Communication with the main server is via HTTP API and Redis events.
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
        hasRepoConfig: !!config.repoConfig,
        hasPreviewConfig: !!config.repoConfig?.previewConfig,
      },
      'Starting sandbox application',
    )

    // Initialize event service with Redis transport for SSE streaming
    let eventService: Awaited<ReturnType<typeof createSandboxEventService>> | undefined
    try {
      eventService = await createSandboxEventService()
      logger.info('Sandbox event service initialized with Redis transport')
    } catch (error) {
      logger.warn(
        { err: error },
        'Failed to initialize event service with Redis, events will not be streamed',
      )
    }

    // Initialize agent request consumer (polls Redis Streams for agent requests)
    const agentConsumer = await createAgentRequestConsumer(config, eventService)
    if (agentConsumer) {
      await agentConsumer.start()
      logger.info('Agent request consumer started')
    } else {
      logger.warn('Agent request consumer not initialized (REDIS_URL not set)')
    }

    // Initialize preview process monitor (watches filesystem for status changes)
    const monitor = initializeMonitor(config, eventService)
    monitor.startWatching()

    const app = createApp(config, eventService)

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
