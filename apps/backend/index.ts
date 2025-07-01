import { config } from 'dotenv'
import fastify from 'fastify'
import { setupApp } from './app.js'

config() // Load environment variables

// Determine if we should disable request logging
// Default to true (disable) unless explicitly set to false
const disableRequestLogging = process.env.DISABLE_REQUEST_LOGGING !== 'false'

const server = fastify({
  disableRequestLogging,
  logger: {
    level: process.env.LOG_LEVEL || 'info',
    transport: {
      target: 'pino-pretty',
      options: {
        translateTime: 'HH:MM:ss Z',
        ignore: 'pid,hostname',
      },
    },
    serializers: {
      // biome-ignore lint/suspicious/noExplicitAny: Fastify serializer types are complex
      req(request: any) {
        return {
          method: request.method,
          url: request.url,
          host: request.headers.host,
          remoteAddress: request.socket.remoteAddress,
          remotePort: request.socket.remotePort,
        }
      },
      // biome-ignore lint/suspicious/noExplicitAny: Fastify serializer types are complex
      res(reply: any) {
        return {
          statusCode: reply.statusCode,
        }
      },
    },
  },
})

const start = async () => {
  try {
    await setupApp(server)

    const port = parseInt(process.env.PORT || '8000', 10)
    const host = process.env.HOST || '0.0.0.0'

    // Setup graceful shutdown handlers
    const gracefulShutdown = async (signal: string) => {
      server.log.info(`Received ${signal}, starting graceful shutdown...`)

      try {
        // Stop all running agents first
        if (server.services?.agent) {
          server.log.info('Stopping all running agents...')
          await server.services.agent.stopAllAgents()
        }

        // Stop preview environment servers
        if (server.services?.previewEnvironment) {
          server.log.info('Shutting down preview environment service...')
          await server.services.previewEnvironment.gracefulShutdown()
        }

        // Close the server
        server.log.info('Closing HTTP server...')
        await server.close()

        server.log.info('Graceful shutdown completed')
        process.exit(0)
      } catch (error) {
        server.log.error('Error during graceful shutdown:', error)
        process.exit(1)
      }
    }

    // Handle shutdown signals
    process.on('SIGTERM', () => gracefulShutdown('SIGTERM'))
    process.on('SIGINT', () => gracefulShutdown('SIGINT'))

    // Handle unhandled promise rejections
    process.on('unhandledRejection', (reason, promise) => {
      server.log.error('Unhandled Rejection at:', promise, 'reason:', reason)
      // Don't exit the process - allow the server to continue running
    })

    // Handle uncaught exceptions
    process.on('uncaughtException', (error) => {
      server.log.error('Uncaught Exception:', error)
      // For uncaught exceptions, we should exit as the process state may be corrupted
      gracefulShutdown('UNCAUGHT_EXCEPTION')
    })

    await server.listen({ port, host })
    console.log(`Server listening on http://${host}:${port}`)
  } catch (err) {
    server.log.error(err)
    process.exit(1)
  }
}

start()
