/**
 * Full-stack Docker Workflow E2E Tests.
 *
 * Tests the complete flow through the Rust backend HTTP API → Docker provisioner
 * → sandbox container → Claude agent. Requires a running Docker Compose stack:
 *
 *   ./docker-compose/dev-docker.sh
 *
 * Prerequisites:
 * - Docker Compose stack running (backend at localhost:8080, PostgreSQL, Redis)
 * - CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY set
 * - At least one repository template in the database
 *
 * Run:
 *   cd tests/docker-compose && bun install && bun test
 */

import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import {
  checkHealth,
  createVibeTask,
  deleteTask,
  ensureRepository,
  getTaskMessages,
  hasAnthropicKey,
  listFiles,
  readFile,
  sendMessage,
  waitForAgentIdle,
  waitForWorkspaceReady,
} from './helpers/backend-client'
import { SSEEventCollector } from './helpers/sse-collector'

// ── Prerequisite checks ─────────────────────────────────────────────

const backendAvailable = await checkHealth()
const hasKey = hasAnthropicKey()
const canRunTests = backendAvailable && hasKey

// ── Test Suite ──────────────────────────────────────────────────────

describe.skipIf(!canRunTests)('Docker Workflow E2E', () => {
  let collector: SSEEventCollector
  let templateId: string

  beforeAll(async () => {
    // Start SSE event collector
    collector = new SSEEventCollector(true)
    collector.start()
    // Give SSE connection a moment to establish
    await sleep(1000)

    // Get or create template repository
    const repo = await ensureRepository()
    templateId = repo.id
    console.log(`Using template: ${templateId} (${repo.name})`)
  }, 30_000)

  afterAll(() => {
    if (collector) {
      collector.stop()
    }
  })

  // ── Single-turn agent execution ─────────────────────────────────

  it('should execute agent and create a file (single-turn)', async () => {
    let taskId: string | undefined

    try {
      // Create task with initial message
      const result = await createVibeTask(
        templateId,
        "Add a heading that says 'E2E Test Success' to the main page",
      )
      taskId = result.taskId
      expect(taskId).toBeTruthy()
      console.log(`Task created: ${taskId}`)

      // Wait for workspace to be ready
      const ready = await waitForWorkspaceReady(taskId)
      expect(ready).toBe(true)
      console.log('Workspace ready')

      // Wait for first taskTurn.completed via SSE
      const turnCompleted = await collector.waitForNthEvent(
        'taskTurn',
        'completed',
        taskId,
        1,
        300_000,
      )

      if (!turnCompleted) {
        // Fallback to polling
        console.log('No SSE event, falling back to polling...')
        const idle = await waitForAgentIdle(taskId, 300_000)
        expect(idle).toBe(true)
      } else {
        console.log('Agent completed (via SSE)')
      }

      // Verify file content
      const match = await findFileWithContent(taskId, 'E2E Test Success')
      expect(match).not.toBeNull()
      console.log(`Content verified in: ${match!.path}`)

      // Verify SSE events
      const taskEvents = collector.getEventsForTask(taskId)
      const hasAgentCompleted = taskEvents.some(
        (e) => e.type === 'agent' && e.subType === 'completed',
      )
      const hasTurnCompleted = taskEvents.some(
        (e) => e.type === 'taskTurn' && e.subType === 'completed',
      )
      const hasAgentMessage = taskEvents.some(
        (e) => e.type === 'agent' && e.subType === 'message',
      )

      expect(hasAgentCompleted).toBe(true)
      expect(hasTurnCompleted).toBe(true)
      expect(hasAgentMessage).toBe(true)
      console.log('SSE events verified')
    } finally {
      if (taskId) {
        await deleteTask(taskId)
        console.log('Task cleaned up')
      }
    }
  }, 600_000)

  // ── Multi-turn session resume ───────────────────────────────────

  it('should resume session on follow-up message (multi-turn)', async () => {
    let taskId: string | undefined

    try {
      // ── Turn 1: Create task and wait for agent completion ──
      const result = await createVibeTask(
        templateId,
        "Add a heading that says 'E2E Test Success' to the main page",
      )
      taskId = result.taskId
      expect(taskId).toBeTruthy()
      console.log(`[Turn 1] Task created: ${taskId}`)

      // Wait for workspace
      const ready = await waitForWorkspaceReady(taskId)
      expect(ready).toBe(true)
      console.log('[Turn 1] Workspace ready')

      // Wait for first taskTurn.completed
      const turn1Completed = await collector.waitForNthEvent(
        'taskTurn',
        'completed',
        taskId,
        1,
        300_000,
      )

      if (!turn1Completed) {
        const idle = await waitForAgentIdle(taskId, 300_000)
        expect(idle).toBe(true)
        console.log('[Turn 1] Agent completed (via polling)')
      } else {
        console.log('[Turn 1] Agent completed (via SSE)')
      }

      // Log session_init from turn 1
      const turn1SessionInit = collector
        .getEventsForTask(taskId)
        .find((e) => e.type === 'agent' && e.subType === 'session_init')
      const turn1SessionId = turn1SessionInit?.payload?.session_id as string | undefined
      console.log(`[Turn 1] Session ID: ${turn1SessionId ?? '(not found)'}`)

      // Verify turn 1 result
      const turn1File = await findFileWithContent(taskId, 'E2E Test Success')
      expect(turn1File).not.toBeNull()
      console.log(`[Turn 1] Content verified in: ${turn1File!.path}`)
      console.log(`[Turn 1] File snippet:\n${truncate(turn1File!.content, 500)}`)

      // Log messages after turn 1
      const turn1Messages = await getTaskMessages(taskId)
      logMessages('[Turn 1]', turn1Messages)

      // ── Turn 2: Send follow-up message (triggers resume) ──
      console.log('\n[Turn 2] Sending follow-up message...')
      await sendMessage(
        taskId,
        "Change the 'E2E Test Success' heading to 'Multi-Turn Success' instead",
      )
      console.log('[Turn 2] Message sent')

      // Wait for SECOND taskTurn.completed
      const turn2Completed = await collector.waitForNthEvent(
        'taskTurn',
        'completed',
        taskId,
        2,
        300_000,
      )

      if (!turn2Completed) {
        console.log('[Turn 2] No SSE event, falling back to polling...')
        const idle = await waitForAgentIdle(taskId, 300_000)
        expect(idle).toBe(true)
        console.log('[Turn 2] Agent completed (via polling)')
      } else {
        console.log('[Turn 2] Agent completed (via SSE)')
      }

      // Log session_init events — turn 2 should have a second one with the resumed session
      const sessionInits = collector
        .getEventsForTask(taskId)
        .filter((e) => e.type === 'agent' && e.subType === 'session_init')
      console.log(`[Turn 2] Total session_init events: ${sessionInits.length}`)
      for (let i = 0; i < sessionInits.length; i++) {
        const si = sessionInits[i]
        console.log(`  session_init[${i + 1}]: session_id=${si.payload.session_id}, model=${si.payload.model}`)
      }

      // Verify turn 2 result — agent should have changed the heading
      const turn2File = await findFileWithContent(taskId, 'Multi-Turn Success')
      expect(turn2File).not.toBeNull()
      console.log(`[Turn 2] Content verified in: ${turn2File!.path}`)
      console.log(`[Turn 2] File snippet:\n${truncate(turn2File!.content, 500)}`)

      // Log messages after turn 2
      const turn2Messages = await getTaskMessages(taskId)
      logMessages('[Turn 2]', turn2Messages)

      // Log full event summary
      logEventSummary(collector, taskId)

      // Verify event counts
      const agentCompletedCount = collector.countEvents('agent', 'completed', taskId)
      const turnCompletedCount = collector.countEvents('taskTurn', 'completed', taskId)

      expect(agentCompletedCount).toBeGreaterThanOrEqual(2)
      expect(turnCompletedCount).toBeGreaterThanOrEqual(2)
    } finally {
      if (taskId) {
        await deleteTask(taskId)
        console.log('Task cleaned up')
      }
    }
  }, 600_000)
})

// ── Skip message when prerequisites not met ─────────────────────────

if (!canRunTests) {
  describe('Docker Workflow E2E', () => {
    if (!backendAvailable) {
      it.skip('Tests skipped — backend not available at localhost:8080', () => {})
    } else if (!hasKey) {
      it.skip('Tests skipped — CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY not set', () => {})
    }
  })
}

// ── Helpers ─────────────────────────────────────────────────────────

interface FileMatch {
  path: string
  content: string
}

/**
 * Search common source file paths for a string, returning the path and content if found.
 */
async function findFileWithContent(
  taskId: string,
  searchString: string,
): Promise<FileMatch | null> {
  // Check common source file locations
  const candidates = [
    'src/pages/Index.tsx',
    'src/App.tsx',
    'src/App.jsx',
    'src/main.tsx',
    'index.html',
  ]

  for (const filePath of candidates) {
    const content = await readFile(taskId, filePath)
    if (content && content.includes(searchString)) {
      return { path: filePath, content }
    }
  }

  // If not found in common paths, try listing src/ recursively
  const files = await listFiles(taskId, 'src', true)
  for (const filePath of files) {
    if (filePath.endsWith('.tsx') || filePath.endsWith('.jsx') || filePath.endsWith('.html')) {
      const content = await readFile(taskId, filePath)
      if (content && content.includes(searchString)) {
        return { path: filePath, content }
      }
    }
  }

  return null
}

/**
 * Log a summary of messages from the API, showing role, type, session ID, and content preview.
 */
function logMessages(prefix: string, messages: Array<{ uuid: string; messageType: string; role: string; sessionId?: string; content: Array<{ type: string; text?: string }>; toolResults?: Array<{ toolName: string }> }>): void {
  console.log(`${prefix} Messages (${messages.length}):`)
  for (const msg of messages) {
    const contentPreview = msg.content
      .filter((b) => b.type === 'text' && b.text)
      .map((b) => truncate(b.text!, 120))
      .join(' | ')
    const tools = msg.toolResults?.map((t) => t.toolName).join(', ')
    const parts = [
      `  ${msg.role}/${msg.messageType}`,
      msg.sessionId ? `session=${msg.sessionId.slice(0, 12)}...` : 'session=none',
    ]
    if (contentPreview) {
      parts.push(`"${contentPreview}"`)
    }
    if (tools) {
      parts.push(`tools=[${tools}]`)
    }
    console.log(parts.join('  '))
  }
}

/**
 * Log a summary of all SSE events for a task.
 */
function logEventSummary(collector: SSEEventCollector, taskId: string): void {
  const events = collector.getEventsForTask(taskId)
  const byType = new Map<string, number>()
  for (const e of events) {
    const key = e.subType ? `${e.type}.${e.subType}` : e.type
    byType.set(key, (byType.get(key) ?? 0) + 1)
  }

  console.log(`\nEvent summary (${events.length} total):`)
  for (const [key, count] of [...byType.entries()].sort()) {
    console.log(`  ${key}: ${count}`)
  }
}

function truncate(s: string, maxLen: number): string {
  const oneLine = s.replace(/\n/g, ' ').trim()
  if (oneLine.length <= maxLen) {
    return oneLine
  }
  return `${oneLine.slice(0, maxLen)}...`
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
