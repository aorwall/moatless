import { existsSync, readdirSync } from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

/** Find a baked Chromium binary, or let Playwright use its installed browser. */
export function bakedChromium(
  root = process.env.PLAYWRIGHT_BROWSERS_PATH ?? '/opt/playwright-browsers',
) {
  let entries
  try {
    entries = readdirSync(root, { withFileTypes: true })
  } catch (error) {
    if (error.code === 'ENOENT') return undefined
    throw error
  }
  const versions = entries
    .filter((entry) => entry.isDirectory() && /^chromium-\d+$/.test(entry.name))
    .map((entry) => entry.name)
    .sort((a, b) => Number(b.split('-')[1]) - Number(a.split('-')[1]))
  for (const version of versions) {
    for (const layout of ['chrome-linux64', 'chrome-linux']) {
      const binary = path.join(root, version, layout, 'chrome')
      if (existsSync(binary)) return binary
    }
  }
  return undefined
}

/** Mint browser auth and read the project names that the UI must display. */
export async function backendFixture(backend, key) {
  const base = new URL(backend)
  if (!['http:', 'https:'].includes(base.protocol)) throw new Error('Backend must use HTTP(S)')
  if (!key) throw new Error('MOATLESS_API_KEY is not set')
  const headers = { authorization: `Bearer ${key}`, 'content-type': 'application/json' }
  const minted = await fetch(new URL('/api/v1/auth/api-key/session', base), {
    method: 'POST',
    headers,
    body: '{}',
    signal: AbortSignal.timeout(15_000),
  })
  if (!minted.ok) throw new Error(`Session request failed: HTTP ${minted.status}`)
  const cookies = minted.headers.getSetCookie?.() ?? [minted.headers.get('set-cookie')]
  const session = /(?:^|\n|;\s*)moatless_session=([^;\n]+)/.exec(cookies.join('\n'))?.[1]
  if (!session) throw new Error('Session response has no moatless_session cookie')
  const listed = await fetch(new URL('/api/v1/tasks?limit=20', base), {
    headers,
    signal: AbortSignal.timeout(15_000),
  })
  if (!listed.ok) throw new Error(`Task request failed: HTTP ${listed.status}`)
  const { items } = await listed.json()
  if (!Array.isArray(items)) throw new Error('Task response has no items array')
  const names = [
    ...new Set(
      items
        .map((task) => task.repositoryName)
        .filter((name) => typeof name === 'string' && name.length > 0),
    ),
  ]
  if (names.length === 0) throw new Error('Backend lists no projects; nothing to assert')
  return { session, names }
}

/** Check an already-running T3 browser harness, closing the browser on every outcome. */
export async function browserCheck(chromium, options, fixture) {
  const { host, port, screenshot, executablePath } = options
  const origin = new URL(`https://${host}/`)
  if (origin.hostname !== host || origin.port || origin.username || origin.password) {
    throw new Error('--host must be a hostname without a scheme, port, or path')
  }
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error('Invalid --port')
  const browser = await chromium.launch({
    executablePath,
    args: [
      '--no-sandbox',
      `--host-resolver-rules=MAP ${host} 127.0.0.1:${port}`,
      '--ignore-certificate-errors',
    ],
  })
  try {
    const context = await browser.newContext({ ignoreHTTPSErrors: true })
    await context.addCookies([
      {
        name: 'moatless_session',
        value: fixture.session,
        url: origin.href,
        secure: true,
        httpOnly: true,
        sameSite: 'Lax',
      },
    ])
    const page = await context.newPage()
    const errors = []
    const sockets = []
    const respondingSockets = new Set()
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message.text())
    })
    page.on('pageerror', (error) => errors.push(error.message))
    page.on('websocket', (socket) => {
      if (new URL(socket.url()).pathname !== '/ws') return
      sockets.push(socket.url())
      socket.on('framereceived', () => respondingSockets.add(socket))
      socket.on('socketerror', (error) => errors.push(`WebSocket: ${error}`))
    })
    const response = await page.goto(origin.href, {
      waitUntil: 'domcontentloaded',
      timeout: 60_000,
    })
    if (!response?.ok())
      throw new Error(`Page request failed: HTTP ${response?.status() ?? 'no response'}`)
    const deadline = Date.now() + 90_000
    let shown
    while (
      Date.now() < deadline &&
      (!shown || respondingSockets.size === 0) &&
      errors.length === 0
    ) {
      const body = await page.locator('body').innerText({ timeout: 5000 })
      shown = fixture.names.find((name) => body.includes(name))
      if (!shown || respondingSockets.size === 0) await page.waitForTimeout(1000)
    }
    await page.screenshot({ path: screenshot })
    const result = {
      sockets: sockets.length,
      respondingSockets: respondingSockets.size,
      project: shown ?? null,
      errors,
    }
    console.log(JSON.stringify(result, null, 2))
    if (sockets.length !== 1 || respondingSockets.size !== 1 || !shown || errors.length > 0)
      throw new Error('Browser integration check failed')
    return result
  } finally {
    await browser.close()
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const args = process.argv.slice(2)
    const values = new Map()
    for (let index = 0; index < args.length; index += 2) {
      const flag = args[index]
      if (
        !['--scratch', '--host', '--port', '--backend', '--screenshot'].includes(flag) ||
        !args[index + 1] ||
        args[index + 1].startsWith('--')
      )
        throw new Error(`Invalid argument: ${flag}`)
      values.set(flag, args[index + 1])
    }
    const scratch = path.resolve(values.get('--scratch') ?? process.env.PW_SCRATCH ?? '/tmp/pw')
    const fixture = await backendFixture(
      values.get('--backend') ?? 'http://localhost:8080',
      process.env.MOATLESS_API_KEY,
    )
    const { chromium } = createRequire(path.join(scratch, 'package.json'))('playwright')
    await browserCheck(
      chromium,
      {
        host: values.get('--host') ?? 'moatless-t3.staging.moatless.xyz',
        port: Number(values.get('--port') ?? 8444),
        screenshot: values.get('--screenshot') ?? path.join(scratch, 'browser-check.png'),
        executablePath: bakedChromium(),
      },
      fixture,
    )
  } catch (error) {
    console.error(error.message)
    process.exitCode = 1
  }
}
