import { config } from 'dotenv'
import fastify from 'fastify'
import { setupApp } from './app.js'

config() // Load environment variables

const server = fastify({
  logger: {
    level: process.env.LOG_LEVEL || 'info',
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
