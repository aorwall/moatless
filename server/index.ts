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
      req(request: any) {
        return {
          method: request.method,
          url: request.url,
          host: request.headers.host,
          remoteAddress: request.socket.remoteAddress,
          remotePort: request.socket.remotePort,
        }
      },
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
        // Stop preview environment servers first
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

    await server.listen({ port, host })
    console.log(`Server listening on http://${host}:${port}`)
  } catch (err) {
    server.log.error(err)
    process.exit(1)
  }
}

start()
