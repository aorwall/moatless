import fastify from 'fastify'
import { setupApp } from '../app.js'

export async function build() {
  const app = fastify({
    logger: false, // Disable logging in tests
  })

  await setupApp(app)
  return app
}