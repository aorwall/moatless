import { expect, type Page, test } from '@playwright/test'

const TASK_ID = 'mock-message-history-task'
const NOW = '2026-07-28T00:00:00.000Z'

test.use({ storageState: { cookies: [], origins: [] } })

type MockMessage = {
  uuid: string
  role: 'user' | 'assistant'
  messageType: 'user' | 'assistant'
  timestamp: string
  content: Array<{ type: 'text'; text: string }>
  toolResults: []
}

function message(
  index: number,
  text = `Message history row ${String(index).padStart(3, '0')}`,
): MockMessage {
  const role = index % 2 === 0 ? 'assistant' : 'user'
  return {
    uuid: `msg-${String(index).padStart(3, '0')}`,
    role,
    messageType: role,
    timestamp: NOW,
    content: [{ type: 'text', text }],
    toolResults: [],
  }
}

function taskResponse() {
  return {
    taskId: TASK_ID,
    repositoryId: 'repo-1',
    templateId: null,
    workspaceId: null,
    name: 'Mock Message History',
    description: null,
    status: 'open',
    createdAt: NOW,
    updatedAt: NOW,
    createdBy: 'test-user',
    createdByDisplayName: 'Test User',
    visibility: 'private',
    agentType: 'codex',
    agentMode: 'edit',
    model: null,
    parentTaskId: null,
    tags: [],
    ttl: null,
    lastReadAt: NOW,
    lastMessageAt: NOW,
  }
}

function sandboxStatus() {
  return {
    sandboxStatus: 'ready',
    sandboxError: null,
    agentStatus: 'idle',
    servers: [],
    scheduledJobs: [],
    backgroundProcesses: [],
  }
}

async function mockTaskPage(
  page: Page,
  options: {
    messages: MockMessage[]
    refreshedMessages?: MockMessage[]
    streamEvents?: unknown[]
  },
) {
  let messageCalls = 0
  let resolveInitialMessages: (() => void) | undefined
  const initialMessagesServed = new Promise<void>((resolve) => {
    resolveInitialMessages = resolve
  })

  await page.route('**/api/**', async (route) => {
    const url = new URL(route.request().url())
    if (!url.pathname.startsWith('/api/')) {
      await route.fallback()
      return
    }
    throw new Error(`Unexpected API request in message-history spec: ${url.toString()}`)
  })

  await page.route('**/api/v1/auth/me', async (route) => {
    await route.fulfill({
      json: {
        user: { id: 'test-user', username: 'test-user', role: 'admin' },
        expiresAt: '2026-07-29T00:00:00.000Z',
        providers: [],
        setup: { complete: true, requirements: [] },
      },
    })
  })

  await page.route('**/api/v1/feature-flags', async (route) => {
    await route.fulfill({ json: {} })
  })

  await page.route('**/api/v1/repositories*', async (route) => {
    await route.fulfill({ json: [] })
  })

  await page.route('**/api/v1/tags', async (route) => {
    await route.fulfill({ json: [] })
  })

  await page.route('**/api/v1/users', async (route) => {
    await route.fulfill({ json: { users: [] } })
  })

  await page.route('**/api/v1/tasks?*', async (route) => {
    await route.fulfill({
      json: {
        items: [],
        pagination: { total: 0, limit: 100, offset: 0 },
      },
    })
  })

  await page.route('**/api/v1/agents', async (route) => {
    await route.fulfill({ json: { agents: [] } })
  })

  await page.route(`**/api/v1/tasks/${TASK_ID}`, async (route) => {
    await route.fulfill({ json: taskResponse() })
  })

  await page.route(`**/api/v1/tasks/${TASK_ID}/read`, async (route) => {
    await route.fulfill({ status: 204 })
  })

  await page.route(`**/api/v1/tasks/${TASK_ID}/connections*`, async (route) => {
    await route.fulfill({ json: [] })
  })

  await page.route(`**/api/v1/tasks/${TASK_ID}/events/history*`, async (route) => {
    await route.fulfill({ json: { items: [] } })
  })

  await page.route(`**/api/v1/tasks/${TASK_ID}/feedback`, async (route) => {
    await route.fulfill({ json: { items: [] } })
  })

  await page.route('**/api/v1/repositories/repo-1/skills', async (route) => {
    await route.fulfill({ json: [] })
  })

  await page.route(`**/api/sandbox/v1/tasks/${TASK_ID}/status`, async (route) => {
    await route.fulfill({ json: sandboxStatus() })
  })

  await page.route(`**/api/sandbox/v1/tasks/${TASK_ID}/files/tree*`, async (route) => {
    await route.fulfill({
      json: {
        gitSummary: { filesChanged: 0, insertions: 0, deletions: 0 },
        tree: [],
      },
    })
  })

  await page.route(`**/api/v1/tasks/${TASK_ID}/messages*`, async (route) => {
    messageCalls += 1
    const messages =
      messageCalls > 1 && options.refreshedMessages ? options.refreshedMessages : options.messages
    await route.fulfill({
      json: {
        messages,
        hasMore: false,
        nextBeforeId: null,
        totalCount: messages.length,
      },
    })
    if (messageCalls === 1) {
      resolveInitialMessages?.()
    }
  })

  await page.route('**/api/v1/events/stream*', async (route) => {
    const events = options.streamEvents ?? []
    await initialMessagesServed
    await new Promise((resolve) => setTimeout(resolve, 50))
    await route.fulfill({
      headers: { 'content-type': 'text/plain; charset=utf-8' },
      body: `${events.map((event) => JSON.stringify(event)).join('\n')}\n`,
    })
  })
}

async function expectPinnedToNewestMessage(page: Page) {
  await expect(page.getByRole('button', { name: 'Scroll to bottom' })).toBeHidden()
}

test.describe('Message History UI', () => {
  test('keeps already-rendered messages when newest-page refresh advances', async ({ page }) => {
    const initial = [1, 2, 3, 4].map((i) =>
      message(i, i === 3 ? 'Gap sentinel 003 should remain visible' : undefined),
    )
    const refreshed = [4, 5, 6].map((i) => message(i))

    await mockTaskPage(page, {
      messages: initial,
      refreshedMessages: refreshed,
      streamEvents: [
        {
          eventId: 'evt-refresh',
          taskId: TASK_ID,
          type: 'taskTurn',
          subType: 'completed',
          isError: false,
          payload: {},
        },
      ],
    })

    await page.goto(`/tasks/${TASK_ID}`)

    await expect(page.getByText('Gap sentinel 003 should remain visible')).toBeVisible()
    await expect(page.getByText('Message history row 006')).toBeVisible()
    await expect(page.getByText('Gap sentinel 003 should remain visible')).toBeVisible()
  })

  test('stays scrolled down when a live message update grows the last row', async ({ page }) => {
    const initial = Array.from({ length: 40 }, (_, idx) => message(idx + 1))
    const expandedLast = message(
      40,
      [
        'Message history row 040',
        'Streamed expansion line that makes the last row much taller.',
        'The viewport should stay pinned to the bottom after this upsert.',
        'Final streamed expansion sentinel.',
      ].join('\n\n'),
    )

    await mockTaskPage(page, {
      messages: initial,
      streamEvents: [
        {
          eventId: 'evt-upsert-last',
          taskId: TASK_ID,
          type: 'message',
          subType: 'upsert',
          isError: false,
          payload: expandedLast,
        },
      ],
    })

    await page.goto(`/tasks/${TASK_ID}`)

    await expect(page.getByText('Message history row 040')).toBeVisible()
    await expectPinnedToNewestMessage(page)
    await expect(page.getByText('Final streamed expansion sentinel.')).toBeVisible()
    await expectPinnedToNewestMessage(page)
  })
})
