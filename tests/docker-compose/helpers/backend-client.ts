/**
 * HTTP client for the Rust backend API.
 *
 * Wraps all endpoints used by the E2E tests with typed responses.
 * Expects the backend to be running at BACKEND_URL (default: http://localhost:8080).
 */

import { BACKEND_URL } from './env'

export interface TaskStatus {
  environmentStatus: string
  workspaceReady: boolean
  agentState: string
  [key: string]: unknown
}

export interface TaskMessage {
  uuid: string
  messageType: string
  role: string
  timestamp: string
  sessionId?: string
  turnNumber?: number
  content: Array<{ type: string; text?: string; [key: string]: unknown }>
  toolResults?: Array<{ toolName: string; [key: string]: unknown }>
  result?: { text?: string; [key: string]: unknown }
  [key: string]: unknown
}

export interface Repository {
  id: string
  name: string
  [key: string]: unknown
}

export interface CreateRepositoryRequest {
  name: string
  kind: 'git' | 'remote' | 'template'
  remoteUrl?: string
  defaultBranch?: string
  dockerImage?: string
  description?: string
}

/**
 * Check if the backend is healthy.
 */
export async function checkHealth(): Promise<boolean> {
  try {
    const resp = await fetch(`${BACKEND_URL}/health`, {
      signal: AbortSignal.timeout(5000),
    })
    if (!resp.ok) {
      return false
    }
    const data = (await resp.json()) as { status: string }
    return data.status === 'ok'
  } catch {
    return false
  }
}

/**
 * Get all repositories.
 */
export async function getRepositories(): Promise<Repository[]> {
  const resp = await fetch(`${BACKEND_URL}/api/v1/repositories`, {
    signal: AbortSignal.timeout(10_000),
  })
  if (!resp.ok) {
    throw new Error(`GET /api/v1/repositories failed: ${resp.status}`)
  }
  return resp.json() as Promise<Repository[]>
}

/**
 * Create a repository (template, git, or remote).
 */
export async function createRepository(req: CreateRepositoryRequest): Promise<Repository> {
  const resp = await fetch(`${BACKEND_URL}/api/v1/repositories`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(req),
    signal: AbortSignal.timeout(10_000),
  })
  if (!resp.ok) {
    const text = await resp.text()
    throw new Error(`POST /api/v1/repositories failed: ${resp.status} — ${text.slice(0, 500)}`)
  }
  return resp.json() as Promise<Repository>
}

/**
 * Ensure at least one repository exists, creating a hello-world-ui template if needed.
 * Returns the repository ID.
 */
export async function ensureRepository(): Promise<{ id: string; name: string }> {
  const repos = await getRepositories()
  if (repos.length > 0) {
    return repos[0]
  }

  console.log('No repositories found, creating hello-world-ui template...')
  const repo = await createRepository({
    name: 'Hello World UI',
    kind: 'git',
    remoteUrl: 'https://github.com/aorwall/hello-world-ui.git',
    defaultBranch: 'main',
  })
  console.log(`Created repository: ${repo.id} (${repo.name})`)
  return repo
}

/**
 * Create a vibe task (provisions sandbox + sends initial message).
 */
export async function createVibeTask(
  templateId: string,
  message: string,
): Promise<{ taskId: string }> {
  const resp = await fetch(`${BACKEND_URL}/api/v1/tasks/vibe`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ templateId, message }),
    signal: AbortSignal.timeout(30_000),
  })
  if (!resp.ok) {
    const text = await resp.text()
    throw new Error(`POST /api/v1/tasks/vibe failed: ${resp.status} — ${text.slice(0, 500)}`)
  }
  return resp.json() as Promise<{ taskId: string }>
}

/**
 * Get task status (sandbox status, agent state, etc.).
 */
export async function getTaskStatus(taskId: string): Promise<TaskStatus> {
  const resp = await fetch(`${BACKEND_URL}/api/sandbox/v1/tasks/${taskId}/status`, {
    signal: AbortSignal.timeout(10_000),
  })
  if (!resp.ok) {
    throw new Error(`GET status failed: ${resp.status}`)
  }
  return resp.json() as Promise<TaskStatus>
}

/**
 * Get task messages.
 */
export async function getTaskMessages(taskId: string): Promise<TaskMessage[]> {
  const resp = await fetch(`${BACKEND_URL}/api/v1/tasks/${taskId}/messages`, {
    signal: AbortSignal.timeout(10_000),
  })
  if (!resp.ok) {
    throw new Error(`GET messages failed: ${resp.status}`)
  }
  const data = (await resp.json()) as { messages: TaskMessage[]; totalCount: number }
  return data.messages
}

/**
 * Send a follow-up message to a task.
 */
export async function sendMessage(taskId: string, message: string): Promise<void> {
  const resp = await fetch(`${BACKEND_URL}/api/v1/tasks/${taskId}/messages`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ message }),
    signal: AbortSignal.timeout(30_000),
  })
  if (!resp.ok) {
    const text = await resp.text()
    throw new Error(`POST message failed: ${resp.status} — ${text.slice(0, 500)}`)
  }
}

/**
 * Read a file from the workspace.
 */
export async function readFile(taskId: string, filePath: string): Promise<string | null> {
  try {
    const params = new URLSearchParams({ path: filePath })
    const resp = await fetch(
      `${BACKEND_URL}/api/sandbox/v1/tasks/${taskId}/files/read?${params}`,
      { signal: AbortSignal.timeout(10_000) },
    )
    if (!resp.ok) {
      return null
    }
    const data = (await resp.json()) as { content: string }
    return data.content
  } catch {
    return null
  }
}

/**
 * List files in the workspace.
 */
export async function listFiles(
  taskId: string,
  dirPath: string,
  recursive = false,
): Promise<string[]> {
  try {
    const params = new URLSearchParams({
      path: dirPath,
      recursive: String(recursive),
    })
    const resp = await fetch(
      `${BACKEND_URL}/api/sandbox/v1/tasks/${taskId}/files/list?${params}`,
      { signal: AbortSignal.timeout(10_000) },
    )
    if (!resp.ok) {
      return []
    }
    const data = (await resp.json()) as { files: string[] }
    return data.files ?? []
  } catch {
    return []
  }
}

/**
 * Delete a task and its sandbox.
 */
export async function deleteTask(taskId: string): Promise<void> {
  try {
    await fetch(`${BACKEND_URL}/api/v1/tasks/${taskId}`, {
      method: 'DELETE',
      signal: AbortSignal.timeout(10_000),
    })
  } catch {
    // Best-effort cleanup
  }
}

/**
 * Wait for workspace to become ready by polling status.
 */
export async function waitForWorkspaceReady(
  taskId: string,
  timeoutMs = 180_000,
): Promise<boolean> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const status = await getTaskStatus(taskId)
      if (status.environmentStatus === 'ready' || status.workspaceReady) {
        return true
      }
    } catch {
      // Not ready yet
    }
    await sleep(2000)
  }
  return false
}

/**
 * Wait for agent to become idle after running.
 */
export async function waitForAgentIdle(taskId: string, timeoutMs = 300_000): Promise<boolean> {
  const deadline = Date.now() + timeoutMs
  let agentStarted = false

  while (Date.now() < deadline) {
    try {
      const status = await getTaskStatus(taskId)

      if (status.agentState === 'running') {
        agentStarted = true
        await sleep(2000)
        continue
      }

      if (status.agentState === 'none' || status.agentState === 'idle') {
        if (agentStarted) {
          return true
        }
        // Check if agent already completed before we started polling
        try {
          const messages = await getTaskMessages(taskId)
          if (messages.length > 1) {
            return true
          }
        } catch {
          // Ignore message fetch errors
        }
      }
    } catch {
      // Transient error
    }
    await sleep(2000)
  }
  return false
}

/**
 * Check if Claude authentication is available.
 */
export function hasAnthropicKey(): boolean {
  return !!process.env['CLAUDE_CODE_OAUTH_TOKEN'] || !!process.env['ANTHROPIC_API_KEY']
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
