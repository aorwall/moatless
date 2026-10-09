/**
 * Write a Playwright storage-state file holding a Moatless browser session,
 * minted from MOATLESS_API_KEY. The key is read from the environment and never
 * printed, so it stays out of tool output, unlike inlining it in `run-code`.
 *
 *   node mint-state.mjs --url https://moatless.example.com --out /tmp/moatless-auth.json
 */

import { writeFileSync } from 'node:fs'
import { parseArgs } from 'node:util'

const { values } = parseArgs({
  options: { url: { type: 'string' }, out: { type: 'string', default: '/tmp/moatless-auth.json' } },
})
const base = values.url ?? process.env.FRONTEND_URL
if (!base) throw new Error('Pass --url <deployment origin> (FRONTEND_URL is unset)')
const key = process.env.MOATLESS_API_KEY
if (!key?.startsWith('mvk_')) throw new Error('MOATLESS_API_KEY is unset or not an mvk_ key')

const response = await fetch(new URL('/api/v1/auth/api-key/session', base), {
  method: 'POST',
  headers: { authorization: `Bearer ${key}` },
  signal: AbortSignal.timeout(15_000),
})
if (!response.ok) throw new Error(`Session exchange failed: HTTP ${response.status}`)

const cookies = response.headers.getSetCookie().map((header) => {
  const [pair, ...attributes] = header.split(';').map((part) => part.trim())
  const [name, ...rest] = pair.split('=')
  const attribute = (key) =>
    attributes.find((a) => a.toLowerCase().startsWith(`${key}=`))?.split('=')[1]
  const flag = (key) => attributes.some((a) => a.toLowerCase() === key)
  const maxAge = Number(attribute('max-age'))
  return {
    name,
    value: rest.join('='),
    // Playwright reads a domain without a leading dot as host-only, which
    // would keep the cookie off the app's own host and every preview host.
    domain: attribute('domain') ? `.${attribute('domain').replace(/^\./, '')}` : new URL(base).hostname,
    path: attribute('path') ?? '/',
    expires: Number.isFinite(maxAge) ? Math.floor(Date.now() / 1000) + maxAge : -1,
    httpOnly: flag('httponly'),
    secure: flag('secure'),
    sameSite: { strict: 'Strict', none: 'None' }[attribute('samesite')?.toLowerCase()] ?? 'Lax',
  }
})
for (const required of ['moatless_session', 'moatless_csrf']) {
  if (!cookies.some((c) => c.name === required)) throw new Error(`No ${required} cookie returned`)
}
const { user } = await response.json()
writeFileSync(values.out, JSON.stringify({ cookies, origins: [] }, null, 2), { mode: 0o600 })
console.log(`Wrote ${values.out}: signed in as ${user?.login ?? 'unknown'}, cookie domain ${cookies[0].domain}`)
