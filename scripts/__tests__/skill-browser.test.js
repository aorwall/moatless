import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  backendFixture,
  bakedChromium,
  browserCheck,
} from '../../.agents/skills/t3-development/references/browser-check.mjs'

afterEach(() => vi.restoreAllMocks())
describe('T3 browser helper', () => {
  it('falls back when the baked browser directory is absent', () => {
    expect(bakedChromium('/does-not-exist/skill-browser-test')).toBeUndefined()
  })
  it('finds an available browser when a newer directory has no executable', () => {
    const root = mkdtempSync(path.join(os.tmpdir(), 'skill-browser-'))
    try {
      mkdirSync(path.join(root, 'chromium-200'))
      const browser = path.join(root, 'chromium-100/chrome-linux64/chrome')
      mkdirSync(path.dirname(browser), { recursive: true })
      writeFileSync(browser, '')
      expect(bakedChromium(root)).toBe(browser)
    } finally {
      rmSync(root, { recursive: true, force: true })
    }
  })
  it.each([401, 500])('rejects a failed session request (%s)', async (status) => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('{}', { status }))
    await expect(backendFixture('http://localhost:8080', 'test')).rejects.toThrow(`HTTP ${status}`)
  })
  // Two `Set-Cookie` headers rather than one comma-joined value, because that is
  // what `issued_session_headers` appends and what `getSetCookie()` splits on.
  const mintResponse = (...cookies) => {
    const headers = new Headers()
    for (const cookie of cookies) headers.append('set-cookie', cookie)
    return new Response('{}', { headers })
  }
  it('rejects task HTTP failures even if the body resembles valid data', async () => {
    vi.spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(
        mintResponse('moatless_session=session; Path=/; Secure', 'moatless_csrf=csrf; Path=/; Secure'),
      )
      .mockResolvedValueOnce(
        new Response('{"items":[{"repositoryName":"example"}]}', { status: 403 }),
      )
    await expect(backendFixture('http://localhost:8080', 'test')).rejects.toThrow(
      'Task request failed: HTTP 403',
    )
  })
  it('rejects a mint that carries no CSRF cookie', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(
      mintResponse('moatless_session=session; Path=/; Secure'),
    )
    await expect(backendFixture('http://localhost:8080', 'test')).rejects.toThrow(
      'Session response has no moatless_csrf cookie',
    )
  })
  it('uses the selected host for its cookie and closes the browser on page failure', async () => {
    const cookies = vi.fn()
    const close = vi.fn()
    const page = {
      on: vi.fn(),
      goto: vi.fn().mockResolvedValue({ ok: () => false, status: () => 502 }),
    }
    const chromium = {
      launch: vi.fn().mockResolvedValue({
        close,
        newContext: async () => ({ addCookies: cookies, newPage: async () => page }),
      }),
    }
    await expect(
      browserCheck(
        chromium,
        { host: 'app.example.org', port: 8444 },
        { session: 'test', csrf: 'csrf', names: ['example'] },
      ),
    ).rejects.toThrow('HTTP 502')
    expect(cookies.mock.calls[0][0][0]).toMatchObject({
      url: 'https://app.example.org/',
      secure: true,
    })
    expect(cookies.mock.calls[0][0][0]).not.toHaveProperty('domain')
    expect(cookies.mock.calls[0][0][1]).toMatchObject({
      name: 'moatless_csrf',
      value: 'csrf',
      url: 'https://app.example.org/',
      httpOnly: false,
    })
    expect(close).toHaveBeenCalledOnce()
  })
  it('verifies real data and closes the browser after success', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {})
    const close = vi.fn()
    const page = {
      on: (event, handler) => {
        if (event === 'websocket')
          handler({
            url: () => 'wss://app.example.org/ws?ticket=test',
            on: (event, callback) => {
              if (event === 'framereceived') callback({ payload: '{}' })
            },
          })
      },
      goto: async () => ({ ok: () => true }),
      locator: () => ({ innerText: async () => 'Projects: fixture-project' }),
      screenshot: vi.fn(),
    }
    const chromium = {
      launch: async () => ({
        close,
        newContext: async () => ({ addCookies: async () => {}, newPage: async () => page }),
      }),
    }
    const result = await browserCheck(
      chromium,
      { host: 'app.example.org', port: 8444, screenshot: '/tmp/example.png' },
      { session: 'test', csrf: 'csrf', names: ['fixture-project'] },
    )
    expect(result).toEqual({
      sockets: 1,
      respondingSockets: 1,
      project: 'fixture-project',
      errors: [],
    })
    expect(page.screenshot).toHaveBeenCalledWith({ path: '/tmp/example.png' })
    expect(close).toHaveBeenCalledOnce()
  })

  it('does not count a rejected WebSocket handshake as a connection', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {})
    const close = vi.fn()
    const page = {
      on: (event, handler) => {
        if (event === 'websocket')
          handler({
            url: () => 'wss://app.example.org/ws',
            on: (name, callback) => {
              if (name === 'socketerror') callback('403 Forbidden')
            },
          })
      },
      goto: async () => ({ ok: () => true }),
      screenshot: async () => {},
    }
    const chromium = {
      launch: async () => ({
        close,
        newContext: async () => ({ addCookies: async () => {}, newPage: async () => page }),
      }),
    }
    await expect(
      browserCheck(
        chromium,
        { host: 'app.example.org', port: 8444 },
        { session: 'test', csrf: 'csrf', names: ['example'] },
      ),
    ).rejects.toThrow('Browser integration check failed')
    expect(close).toHaveBeenCalledOnce()
  })

  it('closes the browser when context setup throws', async () => {
    const close = vi.fn()
    const chromium = {
      launch: async () => ({
        close,
        newContext: async () => {
          throw new Error('context failure')
        },
      }),
    }
    await expect(
      browserCheck(chromium, { host: 'app.example.org', port: 8444 }, {}),
    ).rejects.toThrow('context failure')
    expect(close).toHaveBeenCalledOnce()
  })
})
