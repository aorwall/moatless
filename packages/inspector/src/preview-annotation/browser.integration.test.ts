import * as NodeChildProcess from 'node:child_process'
import * as NodeFS from 'node:fs'
import * as NodeHttp from 'node:http'
import * as NodePath from 'node:path'
import * as NodeURL from 'node:url'
import { type Browser, chromium } from 'playwright-core'
import { afterAll, beforeAll, beforeEach, describe, expect, it } from 'vitest'

import {
  CANCEL_PICK_CHANNEL,
  ELEMENT_PICKED_CHANNEL,
  GUEST_READY_CHANNEL,
  PING_CHANNEL,
  START_PICK_CHANNEL,
} from './protocol.js'

/**
 * The one thing unit tests cannot reach: a real cross-origin frame.
 *
 * Everything about this feature that can break silently lives in the gap
 * between two documents the same process cannot span — a parent that may not
 * address `*`, a guest that must ignore everyone but its host, and an overlay
 * that has to draw inside a page nobody controls. So this stands up two origins
 * on two ports, frames one in the other, and drives the whole session the way
 * the panel does.
 */

/**
 * The overlay's own UI lives in a *closed* shadow root, so nothing outside the
 * runtime can see or drive it — not this test, not the previewed app. What is
 * observable from the page is the host element it mounts, which is enough to
 * say whether a session is up.
 */
const OVERLAY_ATTRIBUTE = 'data-t3code-annotation-ui'
const OVERLAY_COUNT = `document.querySelectorAll('[${OVERLAY_ATTRIBUTE}]').length`

const packageRoot = NodePath.resolve(
  NodePath.dirname(NodeURL.fileURLToPath(import.meta.url)),
  '../..',
)
const bundlePath = NodePath.join(packageRoot, 'dist', 'annotation-client.js')

const GUEST_PAGE = `<!doctype html>
<html><body>
  <main id="content"><button id="target" class="cta">Click me</button></main>
  <script src="/preview-annotation.js"></script>
</body></html>`

/**
 * The host page runs the same three rules `framedAnnotationChannel` does —
 * filter on our envelope, address the frame's own origin, accept only what the
 * frame's window sent — so that what passes here is what the panel would see.
 */
const hostPage = (guestOrigin: string) => `<!doctype html>
<html><body>
  <iframe id="frame" src="${guestOrigin}/" style="width:800px;height:600px;border:0"></iframe>
  <script>
    const SOURCE = "t3code:preview-annotation";
    const frame = document.getElementById("frame");
    const origin = new URL(frame.src).origin;
    window.__received = [];
    window.addEventListener("message", (event) => {
      if (event.source !== frame.contentWindow) return;
      if (event.origin !== origin) return;
      const data = event.data;
      if (typeof data !== "object" || data === null || data.source !== SOURCE) return;
      window.__received.push({ channel: data.channel, args: data.args });
    });
    window.__post = (channel, ...args) => {
      frame.contentWindow.postMessage({ source: SOURCE, channel, args }, origin);
    };
    window.__channels = () => window.__received.map((entry) => entry.channel);
  </script>
</body></html>`

function serve(body: (path: string) => { type: string; content: string | Buffer } | null) {
  const server = NodeHttp.createServer((request, response) => {
    const found = body(new URL(request.url ?? '/', 'http://localhost').pathname)
    if (found === null) {
      response.writeHead(404).end()
      return
    }
    response.writeHead(200, { 'content-type': found.type }).end(found.content)
  })
  return server
}

async function listen(server: NodeHttp.Server, host: string): Promise<number> {
  await new Promise<void>((resolve) => server.listen(0, host, resolve))
  const address = server.address()
  if (address === null || typeof address === 'string') throw new Error('no port')
  return address.port
}

/**
 * `null` where no browser could be launched, which skips the suite.
 *
 * Chromium is not a dependency of a checkout — `playwright-core` ships the
 * driver, not the browser — so this has to degrade rather than fail for a
 * reason unrelated to the code under test. Trying a launch is the only
 * trustworthy question: which build satisfies one (full Chromium, the headless
 * shell, an installed channel) is Playwright's business, not something to
 * second-guess by inspecting paths.
 */
let browser: Browser | null = null
let guestServer: NodeHttp.Server
let hostServer: NodeHttp.Server
let hostUrl: string

beforeAll(async () => {
  try {
    browser = await chromium.launch()
  } catch {
    return
  }
  if (!NodeFS.existsSync(bundlePath)) {
    NodeChildProcess.execFileSync('bun', ['run', 'build:client'], {
      cwd: packageRoot,
      stdio: 'inherit',
    })
  }
  const bundle = NodeFS.readFileSync(bundlePath, 'utf8')

  // Two different hosts, so the frame really is cross-origin: `127.0.0.1` and
  // `localhost` resolve to the same interface and are still separate origins.
  guestServer = serve((path) =>
    path === '/preview-annotation.js'
      ? { type: 'text/javascript', content: bundle }
      : { type: 'text/html', content: GUEST_PAGE },
  )
  const guestPort = await listen(guestServer, '127.0.0.1')
  const guestOrigin = `http://127.0.0.1:${guestPort}`

  hostServer = serve(() => ({ type: 'text/html', content: hostPage(guestOrigin) }))
  const hostPort = await listen(hostServer, 'localhost')
  hostUrl = `http://localhost:${hostPort}/`
}, 120_000)

beforeEach((context) => {
  if (browser === null) context.skip()
})

afterAll(async () => {
  await browser?.close()
  guestServer?.close()
  hostServer?.close()
})

describe('the framed annotation runtime', () => {
  it('announces itself to the document that framed it', async () => {
    const page = await browser!.newPage()
    await page.goto(hostUrl)

    await expect.poll(() => page.evaluate('window.__channels()')).toContain(GUEST_READY_CHANNEL)

    await page.close()
  })

  it('answers a ping from a host that started listening late', async () => {
    const page = await browser!.newPage()
    await page.goto(hostUrl)
    await expect.poll(() => page.evaluate('window.__channels()')).toContain(GUEST_READY_CHANNEL)

    await page.evaluate('window.__received = []')
    await page.evaluate(`window.__post(${JSON.stringify(PING_CHANNEL)})`)

    await expect.poll(() => page.evaluate('window.__channels()')).toContain(GUEST_READY_CHANNEL)

    await page.close()
  })

  it('draws its overlay into the page when the host arms a pick', async () => {
    const page = await browser!.newPage()
    await page.goto(hostUrl)
    await expect.poll(() => page.evaluate('window.__channels()')).toContain(GUEST_READY_CHANNEL)

    await page.evaluate(`window.__post(${JSON.stringify(START_PICK_CHANNEL)}, null)`)

    const frame = page.frames()[1]
    await expect.poll(() => frame?.evaluate(OVERLAY_COUNT)).toBeGreaterThan(0)

    await page.close()
  })

  it('returns what the user picked, described well enough to act on', async () => {
    const page = await browser!.newPage()
    await page.goto(hostUrl)
    await expect.poll(() => page.evaluate('window.__channels()')).toContain(GUEST_READY_CHANNEL)
    await page.evaluate(`window.__post(${JSON.stringify(START_PICK_CHANNEL)}, null)`)

    const frame = page.frames()[1]
    await expect.poll(() => frame?.evaluate(OVERLAY_COUNT)).toBeGreaterThan(0)

    // Driven as a person would: a real click on the page's own button, which
    // selects it and focuses the comment box, then type and submit. The
    // overlay's own controls are behind a closed shadow root and cannot be
    // reached any other way — which is the point of the closed root.
    await frame?.click('#target', { force: true })

    // Selecting focuses the comment box, but on a later tick. Typing before
    // then does not land in a textarea — it lands on the tool hotkeys, where
    // `d`, `e`, `r` and `v` switch tools — so wait for focus to cross into the
    // overlay first. A closed shadow root reports its host as `activeElement`,
    // which is exactly the signal wanted here.
    await expect
      .poll(() => frame?.evaluate(`document.activeElement?.hasAttribute("${OVERLAY_ATTRIBUTE}")`))
      .toBe(true)

    await page.keyboard.type('make this bigger')
    await page.keyboard.press('Control+Enter')

    await expect
      .poll(() => page.evaluate('window.__channels()'), { timeout: 15_000 })
      .toContain(ELEMENT_PICKED_CHANNEL)

    const picked = (await page.evaluate(
      `window.__received.find((entry) => entry.channel === ${JSON.stringify(ELEMENT_PICKED_CHANNEL)}).args[0]`,
    )) as {
      comment: string
      elements: ReadonlyArray<{
        element: { tagName: string; selector: string; htmlPreview: string }
      }>
    } | null

    expect(picked?.comment).toBe('make this bigger')
    expect(picked?.elements).toHaveLength(1)
    expect(picked?.elements[0]?.element.tagName).toBe('button')
    expect(picked?.elements[0]?.element.selector).toBe('#target')
    expect(picked?.elements[0]?.element.htmlPreview).toContain('Click me')

    await page.close()
  }, 30_000)

  it('tears the overlay back down when the host cancels', async () => {
    const page = await browser!.newPage()
    await page.goto(hostUrl)
    await expect.poll(() => page.evaluate('window.__channels()')).toContain(GUEST_READY_CHANNEL)
    await page.evaluate(`window.__post(${JSON.stringify(START_PICK_CHANNEL)}, null)`)

    const frame = page.frames()[1]
    await expect.poll(() => frame?.evaluate(OVERLAY_COUNT)).toBeGreaterThan(0)

    await page.evaluate(`window.__post(${JSON.stringify(CANCEL_PICK_CHANNEL)})`)

    await expect.poll(() => frame?.evaluate(OVERLAY_COUNT)).toBe(0)

    await page.close()
  })

  it('ignores annotation traffic from a window that is not its host', async () => {
    const page = await browser!.newPage()
    await page.goto(hostUrl)
    await expect.poll(() => page.evaluate('window.__channels()')).toContain(GUEST_READY_CHANNEL)

    // A page the guest did not come from — here, the guest's own top-level
    // document opened as a popup — telling it to start picking.
    await page.evaluate(`
      const popup = window.open(document.getElementById("frame").src, "_blank");
      window.__popup = popup;
    `)
    const popup = page.context().pages()[1]
    await popup?.waitForLoadState()
    await page.evaluate(`
      window.__popup.postMessage(
        { source: "t3code:preview-annotation", channel: ${JSON.stringify(START_PICK_CHANNEL)}, args: [null] },
        "*",
      );
    `)

    const frame = page.frames()[1]
    await page.waitForTimeout(500)
    expect(await frame?.evaluate(OVERLAY_COUNT)).toBe(0)

    await popup?.close()
    await page.close()
  })
})
