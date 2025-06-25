import { config } from 'dotenv'
import fastify from 'fastify'
import { setupApp } from './app.js'

config() // Load environment variables

const server = fastify({
  logger: {
    level: process.env.LOG_LEVEL || 'info',
    transport: {
      target: 'pino-pretty',
      options: {
        translateTime: 'HH:MM:ss Z',
        ignore: 'pid,hostname',
      },
    },
    // Custom serializers to control what gets logged
    serializers: {
      req(request) {
        return {
          method: request.method,
          url: request.url,
          host: request.headers.host,
          remoteAddress: request.socket.remoteAddress,
          remotePort: request.socket.remotePort,
        }
      },
      res(reply) {
        return {
          statusCode: reply.statusCode,
        }
      },
    },
    // Custom hooks to control when logging happens
    hooks: {
      // Only log errors for incoming requests
      logMethod(inputArgs, method, level) {
        const [message] = inputArgs
        
        // If it's an incoming request log and not an error, skip it
        if (message === 'incoming request' && level !== 50) { // 50 is error level
          return // Skip logging
        }
        
        // Otherwise, proceed with normal logging
        return method.apply(this, inputArgs)
      },
    },
  },
})

const start = async () => {
  try {
    await setupApp(server)

    const port = parseInt(process.env.PORT || '8000', 10)
    const host = process.env.HOST || '0.0.0.0'

    await server.listen({ port, host })
    console.log(`Server listening on http://${host}:${port}`)
  } catch (err) {
    server.log.error(err)
    process.exit(1)
  }
}

start()
