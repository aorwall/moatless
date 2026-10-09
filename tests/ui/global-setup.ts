import { mkdirSync } from 'node:fs'
import { dirname } from 'node:path'
import { request } from '@playwright/test'
import { storageStatePath, uiEnv } from './helpers/env'

/**
 * Signs the browser in by trading the API key for a session cookie. The login
 * page only offers OAuth on a deployment, which a headless browser cannot
 * complete; the cookie is scoped to the parent domain, so the preview origins
 * the tests open inside the UI are authenticated by it too.
 */
export default async function globalSetup(): Promise<void> {
  const env = uiEnv()
  const context = await request.newContext({ baseURL: env.baseUrl })
  try {
    const minted = await context.post('/api/v1/auth/api-key/session', {
      headers: { authorization: `Bearer ${env.apiKey}` },
    })
    if (!minted.ok()) {
      throw new Error(`API key session exchange failed: HTTP ${minted.status()}`)
    }
    const { user } = (await minted.json()) as { user?: { login?: string } }
    const session = await context.get('/api/auth/session')
    if (!session.ok()) {
      throw new Error(
        `Minted cookie is not accepted by /api/auth/session: HTTP ${session.status()}`,
      )
    }
    mkdirSync(dirname(storageStatePath), { recursive: true })
    await context.storageState({ path: storageStatePath })
    console.log(`[ui setup] ${env.baseUrl} signed in as ${user?.login ?? 'unknown'}`)
  } finally {
    await context.dispose()
  }
}
