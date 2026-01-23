import { execSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const HELLO_WORLD_REPO = 'https://github.com/aorwall/hello-world-ui.git'

/**
 * Docker sandbox test context
 */
export interface DockerSandboxContext {
  /** Root temp directory (for cleanup) */
  rootDir: string
  /** Workspace directory with the cloned repo */
  tempDir: string
  dataDir: string
  containerName: string
  hostPort: number
}

/**
 * Check if Docker is available for testing
 */
export async function isDockerAvailable(): Promise<boolean> {
  try {
    execSync('docker info', { timeout: 5000, stdio: 'ignore' })
    return true
  } catch {
    return false
  }
}

/**
 * Check if the host platform is compatible with the sandbox image
 */
export function isPlatformCompatible(): boolean {
  try {
    const arch = os.arch()
    // Both x64 and arm64 are supported
    return arch === 'x64' || arch === 'arm64'
  } catch {
    return false
  }
}

/**
 * Check if Claude authentication is available for agent tests
 * Checks CLAUDE_CODE_OAUTH_TOKEN (primary) or ANTHROPIC_API_KEY (fallback)
 */
export function hasAnthropicKey(): boolean {
  return !!process.env['CLAUDE_CODE_OAUTH_TOKEN'] || !!process.env['ANTHROPIC_API_KEY']
}

/**
 * Get an available port
 */
async function getAvailablePort(): Promise<number> {
  const { getPort } = await import('@moatless/shell-utils')
  return getPort()
}

/**
 * Set up Docker sandbox test context by cloning the hello-world-ui repo
 */
export async function setupDockerSandbox(): Promise<DockerSandboxContext> {
  // Create root temp directory
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), 'sandbox-docker-test-'))

  // Clone hello-world-ui repository into workspace subdirectory
  const workspaceDir = path.join(rootDir, 'workspace')
  execSync(`git clone --depth 1 ${HELLO_WORLD_REPO} ${workspaceDir}`, { stdio: 'ignore' })

  // Create data directory
  const dataDir = path.join(rootDir, 'data')
  fs.mkdirSync(dataDir, { recursive: true })

  // Generate unique container name
  const containerName = `sandbox-test-${Date.now()}`

  // Get available port
  const hostPort = await getAvailablePort()

  return {
    rootDir,
    tempDir: workspaceDir,
    dataDir,
    containerName,
    hostPort,
  }
}

/**
 * Get the sandbox Docker image to use
 * Uses SANDBOX_DOCKER_IMAGE env var if set, otherwise defaults to GHCR
 */
export function getSandboxImage(): string {
  return process.env['SANDBOX_DOCKER_IMAGE'] || 'ghcr.io/aorwall/moatless-vibe-sandbox-js:latest'
}

/**
 * Start Docker sandbox container
 */
export async function startDockerSandbox(
  ctx: DockerSandboxContext,
  image = getSandboxImage(),
): Promise<void> {
  const { tempDir, dataDir, containerName, hostPort } = ctx

  // Build docker create command
  let cmd = `docker create --name "${containerName}"`
  cmd += ` -v "${tempDir}:${tempDir}"`
  cmd += ` -v "${dataDir}:/opt/moatless/data"`
  cmd += ` -w "${tempDir}"`
  cmd += ` -p ${hostPort}:8001`
  cmd += ` -e "WORKSPACE_DIR=${tempDir}"`
  cmd += ` -e "DATA_DIR=/opt/moatless/data"`
  cmd += ` -e "PORT=8001"`
  cmd += ` -e "LOG_LEVEL=info"`
  cmd += ` -e "TASK_ID=${containerName}"`
  cmd += ` -e "TASK_AUTH_TOKEN=test-token-for-integration-tests"`
  cmd += ` -e "BACKEND_API_URL=http://localhost:8000"`

  // Add Claude token if available
  if (process.env['CLAUDE_CODE_OAUTH_TOKEN']) {
    cmd += ` -e "CLAUDE_CODE_OAUTH_TOKEN=${process.env['CLAUDE_CODE_OAUTH_TOKEN']}"`
  }

  // Add Anthropic API key if available
  if (process.env['ANTHROPIC_API_KEY']) {
    cmd += ` -e "ANTHROPIC_API_KEY=${process.env['ANTHROPIC_API_KEY']}"`
  }

  cmd += ` ${image}`

  // Create container
  execSync(cmd, { stdio: 'ignore' })

  // Start container
  execSync(`docker start ${containerName}`, { stdio: 'ignore' })

  // Wait for health check
  await waitForContainerHealth(hostPort, 60000)
}

/**
 * Stop and remove Docker sandbox container
 */
export async function stopDockerSandbox(ctx: DockerSandboxContext): Promise<void> {
  const { containerName, rootDir } = ctx

  try {
    execSync(`docker stop ${containerName}`, { stdio: 'ignore', timeout: 10000 })
  } catch {
    // Ignore
  }

  try {
    execSync(`docker rm ${containerName}`, { stdio: 'ignore', timeout: 5000 })
  } catch {
    // Ignore
  }

  // Remove root temp directory (includes workspace and data)
  if (rootDir && fs.existsSync(rootDir)) {
    fs.rmSync(rootDir, { recursive: true, force: true })
  }
}

/**
 * Wait for container to respond to health checks
 */
async function waitForContainerHealth(hostPort: number, timeoutMs: number): Promise<void> {
  const startTime = Date.now()
  const healthUrl = `http://localhost:${hostPort}/health`

  while (Date.now() - startTime < timeoutMs) {
    try {
      const response = await fetch(healthUrl, {
        signal: AbortSignal.timeout(5000),
      })
      if (response.ok) {
        return
      }
    } catch {
      // Container not ready yet
    }
    await new Promise((resolve) => setTimeout(resolve, 1000))
  }

  throw new Error(`Container health check timed out after ${timeoutMs}ms`)
}

/**
 * Create HTTP client for Docker sandbox
 */
export function createDockerSandboxClient(hostPort: number) {
  const baseUrl = `http://localhost:${hostPort}`

  return {
    baseUrl,

    get: async (path: string) => {
      const response = await fetch(`${baseUrl}${path}`)
      return {
        status: response.status,
        ok: response.ok,
        json: () => response.json(),
        text: () => response.text(),
      }
    },

    post: async (path: string, body?: unknown) => {
      const response = await fetch(`${baseUrl}${path}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: body ? JSON.stringify(body) : undefined,
      })
      return {
        status: response.status,
        ok: response.ok,
        json: () => response.json(),
        text: () => response.text(),
      }
    },

    delete: async (path: string) => {
      const response = await fetch(`${baseUrl}${path}`, {
        method: 'DELETE',
      })
      return {
        status: response.status,
        ok: response.ok,
        json: () => response.json(),
        text: () => response.text(),
      }
    },

    /**
     * Execute agent and wait for completion via SSE
     */
    executeAgent: async (
      message: string,
      options: { maxEvents?: number; timeoutMs?: number } = {},
    ): Promise<{ events: AgentEvent[]; success: boolean }> => {
      const { maxEvents = 100, timeoutMs = 90000 } = options
      const events: AgentEvent[] = []

      const response = await fetch(`${baseUrl}/stream-events`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ message }),
      })

      if (!response.ok) {
        const errorText = await response.text()
        throw new Error(`Agent execution failed: ${response.status} - ${errorText}`)
      }

      const reader = response.body?.getReader()
      if (!reader) {
        throw new Error('No response body')
      }

      const decoder = new TextDecoder()
      const startTime = Date.now()
      let buffer = ''
      let success = false

      try {
        while (events.length < maxEvents && Date.now() - startTime < timeoutMs) {
          const { done, value } = await reader.read()
          if (done) break

          buffer += decoder.decode(value, { stream: true })

          // Parse SSE events from buffer
          const lines = buffer.split('\n')
          buffer = lines.pop() || ''

          let currentEvent = ''
          let currentData = ''

          for (const line of lines) {
            if (line.startsWith('event:')) {
              currentEvent = line.slice(6).trim()
            } else if (line.startsWith('data:')) {
              currentData = line.slice(5).trim()
            } else if (line === '' && currentEvent && currentData) {
              try {
                const parsedData = JSON.parse(currentData)
                events.push({ event: currentEvent, data: parsedData })

                // Check for completion events
                if (currentEvent === 'done' || currentEvent === 'error') {
                  success = currentEvent === 'done'
                  await reader.cancel()
                  return { events, success }
                }

                // Also check agent state in the data
                if (
                  parsedData.state === 'completed' ||
                  parsedData.state === 'stopped' ||
                  parsedData.state === 'error'
                ) {
                  success = parsedData.state === 'completed'
                  await reader.cancel()
                  return { events, success }
                }
              } catch {
                events.push({ event: currentEvent, data: currentData })
              }
              currentEvent = ''
              currentData = ''
            }
          }
        }
      } finally {
        try {
          await reader.cancel()
        } catch {
          // Ignore cancel errors
        }
      }

      return { events, success }
    },
  }
}

interface AgentEvent {
  event: string
  data: unknown
}
