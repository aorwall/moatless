import { serve } from '@hono/node-server'
import { createApp } from './app'
import { loadConfig } from './config/environment'
import { createMessagePollingConsumer } from './features/agents/services/message-polling-consumer'
import { createSandboxEventService } from './features/events/services/sandbox-event.service'
import { initializeServerRegistry } from './features/servers'
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
 * Server processes are managed externally by the entrypoint script
 * and the Claude Code agent via /opt/moatless/bin/server <id> <action>.
 * This app monitors their state via filesystem (PID, status, log files).
 *
 * Communication with the main server is via HTTP API and Redis events.
 * Agent requests are received by polling the backend API for pending messages.
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

    // Initialize message polling consumer (polls backend API for pending messages)
    const messageConsumer = createMessagePollingConsumer(config, eventService)
    await messageConsumer.start()
    logger.info('Message polling consumer started')

    // Initialize server registry (manages all server monitors including file-explorer)
    const serverRegistry = initializeServerRegistry(config, eventService)
    logger.info(
      { servers: (await serverRegistry.getAllStatuses()).map((s) => s.name) },
      'Server registry initialized',
    )

    const app = createApp(config, { eventService, messageConsumer })

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
