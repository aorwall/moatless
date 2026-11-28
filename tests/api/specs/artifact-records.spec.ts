/**
 * Chain: Artifact records
 *
 * A page published with `--data --notify` collects what its readers save and
 * tells the Task's agent. The page runs under a CSP that lets it reach only the
 * backend, a burst of saves reaches the agent as one notice, and the agent's
 * `moat messages reply` to that notice lands on the open page as a record.
 *
 * Chain:
 *   ART-01 → ART-02 → ART-03 → ART-04 → ART-05
 *
 * Scenarios:
 *   ART-01: The page is served under the page CSP, configured for its reader
 *   ART-02: The page's record socket opens and sends a snapshot
 *   ART-03: Five saves from the page queue one notice that lists them all
 *   ART-04: A reply to the notice is saved as a record the open page receives
 *   ART-05: A page published with --private is reopened by --public
 */

import { writeFileSync } from 'node:fs'
import type { UiMessage } from '@moatless/api-client/sandbox/model'
import { missingCapability } from '../../shared/profile'
import { scopedSlug } from '../../shared/run-scope'
import { expect, test } from '../fixtures/api-fixtures'
import { progress } from '../helpers/progress'
import { discardTask } from '../helpers/teardown'

const reason = missingCapability('artifactPages')

/** What the served page's `moatless.artifact` script is configured with. */
interface PageConfig {
  token: string
  artifactId: string
  viewer: string
  viewerId: string
}

/** One frame from the record socket, as `data_script.js` reads it. */
interface SocketFrame {
  _tag: string
  requestId?: string
  values?: { _tag: string; items?: { key: string }[]; done?: boolean }[]
}

/**
 * The page's record socket, collecting every frame it receives so a test can
 * wait for one that has not arrived yet.
 */
class RecordSocket {
  private readonly frames: SocketFrame[] = []
  private readonly socket: WebSocket

  constructor(url: string) {
    this.socket = new WebSocket(url)
    this.socket.onmessage = (message) => {
      const parsed = JSON.parse(String(message.data)) as SocketFrame | SocketFrame[]
      this.frames.push(...(Array.isArray(parsed) ? parsed : [parsed]))
    }
  }

  opened(): Promise<void> {
    return new Promise((resolve, reject) => {
      this.socket.onopen = () => resolve()
      this.socket.onerror = () => reject(new Error('the record socket failed to open'))
    })
  }

  subscribe(id: string, prefix: string): void {
    this.socket.send(
      JSON.stringify({
        _tag: 'Request',
        id,
        tag: 'artifact.records.subscribe',
        payload: { prefix },
      }),
    )
  }

  /** The first value of `tag` pushed for `requestId` that `match` accepts. */
  async waitFor(
    requestId: string,
    tag: string,
    match: (value: NonNullable<SocketFrame['values']>[number]) => boolean,
    timeoutMs = 15_000,
  ): Promise<NonNullable<SocketFrame['values']>[number] | null> {
    const deadline = Date.now() + timeoutMs
    while (Date.now() < deadline) {
      for (const frame of this.frames) {
        if (frame._tag !== 'Chunk' || frame.requestId !== requestId) continue
        const value = frame.values?.find((v) => v._tag === tag && match(v))
        if (value) return value
      }
      await new Promise((resolve) => setTimeout(resolve, 100))
    }
    return null
  }

  close(): void {
    this.socket.close()
  }
}

function pageConfig(html: string): PageConfig | null {
  const match = /var c=(\{.*?\}),b="\/api\/artifact-data\//.exec(html)
  return match ? (JSON.parse(match[1]) as PageConfig) : null
}

function messageText(message: UiMessage): string {
  return message.content.map((block) => block.text ?? '').join('\n')
}

test.describe('Chain: Artifact records', () => {
  test.describe.configure({ mode: 'serial' })
  test.skip(reason !== null, reason ?? '')

  let taskId: string | undefined
  let slug: string
  let config: PageConfig
  let socket: RecordSocket | undefined
  let noticeId: string

  test.beforeAll(async ({ api }) => {
    const workspace = await api.ensureTemplateWorkspace()
    const task = await api.createVibeTask({ workspaceId: workspace.id, name: 'artifact-records' })
    taskId = task.taskId
  })

  test.afterAll(async ({ api }) => {
    socket?.close()
    await discardTask(api, taskId)
  })

  // ─── ART-01: The page is served under the page CSP ──────────────────

  test('ART-01: The page is served under the page CSP, configured for its reader', async ({
    api,
  }) => {
    slug = scopedSlug('survey')
    const path = test.info().outputPath('survey.html')
    writeFileSync(
      path,
      '<!doctype html><html><head><title>Survey</title></head><body></body></html>',
    )
    const published = await api.publishArtifact(taskId!, path, {
      title: 'Survey',
      slug,
      data: true,
      notify: true,
    })
    expect(published.notifyOnData, 'the page was published without --notify').toBe(true)

    const page = await api.cli.raw('GET', published.url)
    expect(page.status, `the published page did not load: ${JSON.stringify(page.body)}`).toBe(200)

    const csp = page.headers['content-security-policy'] ?? ''
    const origin = new URL(api.profile.baseUrl)
    expect(csp, 'the page may reach hosts other than the CDNs and the backend').toContain(
      "default-src 'none'",
    )
    expect(csp, "the page's download links do nothing").toContain('allow-downloads')
    expect(csp, 'the page cannot reach the backend over http').toContain(origin.origin)
    const socketScheme = origin.protocol === 'https:' ? 'wss:' : 'ws:'
    expect(csp, "the page cannot open the backend's record socket").toContain(
      `${socketScheme}//${origin.host}`,
    )

    const parsed = pageConfig(String(page.body))
    expect(parsed, 'the served page carries no moatless.artifact config').not.toBeNull()
    config = parsed!
    expect(config.viewerId, "the page's viewerId is not the reader's user id").toBe(
      await api.userId(),
    )
    expect(config.token, 'the page carries no token').toBeTruthy()
    progress(`published ${slug} v${published.version}`)
  })

  // ─── ART-02: The record socket opens ────────────────────────────────

  test("ART-02: The page's record socket opens and sends a snapshot", async ({ api }) => {
    const url = new URL(`/api/artifact-data/${config.token}/ws`, api.profile.baseUrl)
    url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
    socket = new RecordSocket(url.href)
    await socket.opened()
    socket.subscribe('1', 'agent-reply/')

    const snapshot = await socket.waitFor('1', 'Snapshot', (value) => value.done === true)
    expect(snapshot, 'the socket opened but sent no snapshot').not.toBeNull()
    expect(snapshot!.items, 'a new Artifact already holds agent replies').toEqual([])
  })

  // ─── ART-03: Five saves queue one notice ────────────────────────────

  test('ART-03: Five saves from the page queue one notice that lists them all', async ({ api }) => {
    for (const n of [1, 2, 3, 4, 5]) {
      await api.writePageRecord(config.token, { op: 'save', key: `vote/${n}`, data: { n } })
    }

    const { messages } = await api.getTaskMessages(taskId!)
    const notices = messages.filter((m) => m.externalSource?.adapterKind === 'artifact')
    expect(notices, 'five quick saves did not make exactly one notice').toHaveLength(1)
    const text = messageText(notices[0])
    expect(text, 'the notice does not count the five records').toContain(
      `A reader wrote 5 records on the Artifact "Survey" (\`${slug}\`)`,
    )
    expect(text, 'the notice does not list every key').toContain(
      '`vote/1`, `vote/2`, `vote/3`, `vote/4`, `vote/5`',
    )
    noticeId = notices[0].uuid
  })

  // ─── ART-04: A reply lands on the open page ─────────────────────────

  test('ART-04: A reply to the notice is saved as a record the open page receives', async ({
    api,
  }) => {
    const key = `agent-reply/${noticeId}`
    const reply = await api.replyToMessage(noticeId, 'Thanks, counted.')
    expect(reply.externalId, 'the reply was not saved under the notice id').toBe(key)

    const pushed = await socket!.waitFor(
      '1',
      'Changes',
      (value) => value.items?.some((item) => item.key === key) ?? false,
    )
    expect(pushed, 'the open page was not sent the reply').not.toBeNull()

    const record = await api.getArtifactRecord(taskId!, slug, key)
    expect(record.data, 'the reply record does not hold the reply').toEqual({
      text: 'Thanks, counted.',
      messageId: noticeId,
    })
  })

  // ─── ART-05: --public reopens a private page ────────────────────────

  test('ART-05: A page published with --private is reopened by --public', async ({ api }) => {
    const reopened = scopedSlug('reopened')
    const path = test.info().outputPath('reopened.html')
    writeFileSync(path, '<!doctype html><html><head><title>Reopened</title></head></html>')
    const page = { title: 'Reopened', slug: reopened }

    const closed = await api.publishArtifact(taskId!, path, { ...page, visibility: 'private' })
    expect(closed.visibility, '--private did not close the page').toBe('private')
    const kept = await api.publishArtifact(taskId!, path, page)
    expect(kept.visibility, 'a republish naming no visibility reopened it').toBe('private')
    await api.publishArtifact(taskId!, path, { ...page, visibility: 'public' })

    const listed = (await api.listArtifacts(taskId!)).find((a) => a.slug === reopened)
    expect(listed?.visibility, 'the listing does not show the page public').toBe('public')
  })
})
