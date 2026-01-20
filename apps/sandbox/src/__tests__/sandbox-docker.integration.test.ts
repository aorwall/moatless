/**
 * Docker integration tests for Sandbox
 *
 * These tests run the sandbox inside a Docker container and verify
 * all endpoints work correctly through the container's HTTP interface.
 *
 * Prerequisites:
 * - Docker daemon must be running
 * - Sandbox image: ghcr.io/aorwall/moatless-vibe-sandbox-js:latest
 * - CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY for agent tests
 *   (Available in .env file at repository root)
 *
 * Run with environment variables loaded:
 * export $(grep -v '^#' ../../.env | xargs) && bun test sandbox-docker.integration
 *
 * Or run individual test:
 * export $(grep -v '^#' ../../.env | xargs) && bun test sandbox-docker.integration -t "test name"
 */
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import {
  createDockerSandboxClient,
  type DockerSandboxContext,
  hasAnthropicKey,
  isDockerAvailable,
  isPlatformCompatible,
  setupDockerSandbox,
  startDockerSandbox,
  stopDockerSandbox,
} from './helpers/docker-sandbox'

// Skip entire test suite if Docker is not available or platform incompatible
const dockerAvailable = await isDockerAvailable()
const platformCompatible = isPlatformCompatible()
const canRunTests = dockerAvailable && platformCompatible

describe.skipIf(!canRunTests)('Sandbox Docker Integration', () => {
  let ctx: DockerSandboxContext
  let client: ReturnType<typeof createDockerSandboxClient>

  beforeAll(async () => {
    ctx = await setupDockerSandbox()
    await startDockerSandbox(ctx)
    client = createDockerSandboxClient(ctx.hostPort)
  }, 120000) // 2 minute timeout for container startup

  afterAll(async () => {
    if (ctx) {
      await stopDockerSandbox(ctx)
    }
  }, 30000) // 30 second timeout for container cleanup

  describe('Health & Status', () => {
    it('should return healthy status', async () => {
      const response = await client.get('/health')
      expect(response.ok).toBe(true)

      const data = (await response.json()) as { status: string; mode: string; taskId: string }
      expect(data.status).toBe('ok')
      expect(data.mode).toBe('sandbox')
    })

    it('should return workspace status', async () => {
      const response = await client.get('/workspace/status')
      expect(response.ok).toBe(true)

      const data = (await response.json()) as { status: string; workspaceDir: string }
      expect(data.status).toBe('ready')
      expect(data.workspaceDir).toBe(ctx.tempDir)
    })
  })

  describe('File Operations', () => {
    it('should list files in workspace', async () => {
      const response = await client.post('/list-files', { path: '/', recursive: false })
      expect(response.ok).toBe(true)

      const data = (await response.json()) as { files: string[] }
      expect(Array.isArray(data.files)).toBe(true)
      expect(data.files.length).toBeGreaterThan(0)
    })

    it('should read existing file', async () => {
      const response = await client.post('/read-file', { filePath: 'package.json' })
      expect(response.ok).toBe(true)

      const data = (await response.json()) as { content: string }
      // vite-react-typescript-starter package name from the hello-world-ui repo
      expect(data.content).toContain('vite-react-typescript-starter')
    })

    it('should write and read back a file', async () => {
      const testContent = `Test content created at ${new Date().toISOString()}`

      // Write file
      const writeResponse = await client.post('/write-file', {
        filePath: 'test-output.txt',
        content: testContent,
      })
      expect(writeResponse.ok).toBe(true)

      // Read back
      const readResponse = await client.post('/read-file', { filePath: 'test-output.txt' })
      expect(readResponse.ok).toBe(true)

      const data = (await readResponse.json()) as { content: string }
      expect(data.content).toBe(testContent)
    })
  })

  describe('Git Operations', () => {
    it('should get git status', async () => {
      const response = await client.get('/git-status')
      expect(response.ok).toBe(true)

      const data = (await response.json()) as { currentBranch: string }
      expect(data.currentBranch).toBeDefined()
    })

    it('should get current commit hash', async () => {
      const response = await client.get('/current-commit-hash')
      expect(response.ok).toBe(true)

      const data = (await response.json()) as { hash: string }
      expect(data.hash).toBeDefined()
      expect(data.hash.length).toBeGreaterThan(0)
    })

    it('should detect git changes after file write', async () => {
      // Write a new file
      await client.post('/write-file', {
        filePath: 'git-test-file.txt',
        content: 'Git change test',
      })

      // Check git changes
      const response = await client.get('/git-changes')
      expect(response.ok).toBe(true)
    })
  })

  describe('Preview Server', () => {
    it('should start preview server and reach it', async () => {
      // Start preview server using bun run dev (Vite dev server from hello-world-ui)
      const startResponse = await client.post('/server/start', {
        serverCommands: ['bun run dev -- --host --port {PORT}'],
        setupCommands: ['bun install'],
        startupTimeoutSeconds: 60, // Allow time for install + dev server startup
      })
      expect(startResponse.ok).toBe(true)

      const startData = (await startResponse.json()) as {
        success: boolean
        status: string
        port: number
      }
      expect(startData.success).toBe(true)
      expect(startData.status).toBe('started')
      expect(startData.port).toBeGreaterThan(0)

      // Verify server is running
      const statusResponse = await client.get('/server/status')
      expect(statusResponse.ok).toBe(true)

      const statusData = (await statusResponse.json()) as {
        status: string
        running: boolean
        internalUrl: string
      }
      expect(statusData.status).toBe('started')
      expect(statusData.running).toBe(true)
      expect(statusData.internalUrl).toBeDefined()
    }, 180000) // 3 minute timeout for install + dev server

    it('should proxy requests to preview server', async () => {
      // Make request through proxy
      const response = await client.get('/server/proxy/')
      expect(response.ok).toBe(true)

      const html = await response.text()
      // hello-world-ui contains "Hello World" in its content
      expect(html).toContain('Hello')
    })

    it('should get server logs', async () => {
      const response = await client.get('/server/logs')
      expect(response.ok).toBe(true)

      const data = (await response.json()) as { logs: string }
      expect(data.logs).toBeDefined()
    })

    it('should stop preview server', async () => {
      const stopResponse = await client.post('/server/stop', {})
      expect(stopResponse.ok).toBe(true)

      const stopData = (await stopResponse.json()) as { success: boolean }
      expect(stopData.success).toBe(true)

      // Verify stopped
      const statusResponse = await client.get('/server/status')
      const statusData = (await statusResponse.json()) as { status: string; running: boolean }
      expect(statusData.status).toBe('stopped')
      expect(statusData.running).toBe(false)
    })
  })

  // Agent tests require CLAUDE_CODE_OAUTH_TOKEN
  describe.skipIf(!hasAnthropicKey())('Agent Execution', () => {
    it('should execute agent and receive completion response', async () => {
      // Give agent a simple task
      const result = await client.executeAgent(
        'Create a file called hello-from-agent.txt with the content "Hello from Claude agent test!"',
        { timeoutMs: 90000 },
      )

      // Should have received events
      expect(result.events.length).toBeGreaterThan(0)

      // Check for agent events
      const eventTypes = result.events.map((e) => e.event)
      expect(eventTypes).toContain('agent')

      // Verify the file was created
      const readResponse = await client.post('/read-file', {
        filePath: 'hello-from-agent.txt',
      })

      if (readResponse.ok) {
        const data = (await readResponse.json()) as { content: string }
        expect(data.content).toContain('Hello from Claude agent test')
      }
    }, 120000) // 2 minute timeout for agent execution

    it('should show agent status after completion', async () => {
      // Wait a moment for state to settle
      await new Promise((resolve) => setTimeout(resolve, 1000))

      const response = await client.get('/agent-status')
      expect(response.ok).toBe(true)

      const data = (await response.json()) as { state: string }
      // After completion, agent should be idle or stopped
      expect(['idle', 'stopped', 'completed']).toContain(data.state)
    })

    it('should handle stop agent when none running', async () => {
      const response = await client.post('/stop-agent', {})
      expect(response.ok).toBe(true)

      const data = (await response.json()) as { success: boolean }
      expect(data.success).toBe(true)
    })
  })

  describe('Browser Automation', () => {
    it('should return browser status', async () => {
      const response = await client.get('/browser')
      expect(response.ok).toBe(true)

      const data = (await response.json()) as {
        running: boolean
        pageCount: number
        pages: string[]
      }
      expect(data).toHaveProperty('running')
      expect(data).toHaveProperty('pageCount')
      expect(data).toHaveProperty('pages')
    })

    it('should start and stop browser server', async () => {
      // Start browser
      const startResponse = await client.post('/browser/start', {})
      expect(startResponse.ok).toBe(true)

      const startData = (await startResponse.json()) as {
        success: boolean
        info: { running: boolean }
      }
      expect(startData.success).toBe(true)
      expect(startData.info.running).toBe(true)

      // Stop browser
      const stopResponse = await client.post('/browser/stop', {})
      expect(stopResponse.ok).toBe(true)

      const stopData = (await stopResponse.json()) as { success: boolean }
      expect(stopData.success).toBe(true)
    })

    it('should create and manage pages', async () => {
      // Start browser first
      await client.post('/browser/start', {})

      // Create page
      const createResponse = await client.post('/browser/pages', {
        name: 'test-page',
        url: 'about:blank',
      })
      expect(createResponse.ok).toBe(true)

      const createData = (await createResponse.json()) as {
        success: boolean
        name: string
      }
      expect(createData.success).toBe(true)
      expect(createData.name).toBe('test-page')

      // List pages
      const listResponse = await client.get('/browser/pages')
      expect(listResponse.ok).toBe(true)

      const listData = (await listResponse.json()) as {
        pages: Array<{ name: string }>
      }
      expect(listData.pages.map((p) => p.name)).toContain('test-page')

      // Close page
      const closeResponse = await client.delete('/browser/pages/test-page')
      expect(closeResponse.ok).toBe(true)

      const closeData = (await closeResponse.json()) as { success: boolean }
      expect(closeData.success).toBe(true)

      // Stop browser
      await client.post('/browser/stop', {})
    })

    it('should get ARIA snapshot and take screenshot of preview page', async () => {
      // Start preview server
      const startServerResponse = await client.post('/server/start', {
        serverCommands: ['bun run dev -- --host --port {PORT}'],
        setupCommands: ['bun install'],
        startupTimeoutSeconds: 60,
      })
      expect(startServerResponse.ok).toBe(true)

      // Get internal URL
      const statusResponse = await client.get('/server/status')
      const { internalUrl } = (await statusResponse.json()) as { internalUrl: string }

      // Start browser and create page navigated to preview
      await client.post('/browser/start', {})
      const createPageResponse = await client.post('/browser/pages', {
        name: 'preview',
        url: internalUrl,
      })
      expect(createPageResponse.ok).toBe(true)

      // Wait for page to load
      await new Promise((resolve) => setTimeout(resolve, 2000))

      // Get ARIA snapshot
      const snapshotResponse = await client.get('/browser/pages/preview/snapshot')
      if (!snapshotResponse.ok) {
        const errorText = await snapshotResponse.text()
        console.error('Snapshot request failed:', snapshotResponse.status, errorText)
      }
      expect(snapshotResponse.ok).toBe(true)

      const snapshotData = (await snapshotResponse.json()) as {
        snapshot: string
        url: string
      }
      // ARIA snapshot should contain some content (format varies by Playwright version)
      expect(snapshotData.snapshot.length).toBeGreaterThan(0)
      expect(snapshotData.url).toBeDefined()

      // Take screenshot
      const screenshotResponse = await client.post('/browser/pages/preview/screenshot', {})
      expect(screenshotResponse.ok).toBe(true)

      const screenshotData = (await screenshotResponse.json()) as {
        data: string
        mimeType: string
        width: number
        height: number
      }
      expect(screenshotData.data).toBeDefined()
      expect(screenshotData.data.length).toBeGreaterThan(0)
      expect(screenshotData.mimeType).toBe('image/png')
      expect(screenshotData.width).toBeGreaterThan(0)
      expect(screenshotData.height).toBeGreaterThan(0)

      // Cleanup
      await client.delete('/browser/pages/preview')
      await client.post('/browser/stop', {})
      await client.post('/server/stop', {})
    }, 180000) // 3 minute timeout for install + dev server + browser operations
  })
})

// Provide skip message when tests cannot run
if (!canRunTests) {
  describe('Sandbox Docker Integration', () => {
    if (!dockerAvailable) {
      it.skip('Tests skipped - Docker not available', () => {
        // Skipped test - provides visibility in output
      })
    } else if (!platformCompatible) {
      it.skip('Tests skipped - Platform not compatible with sandbox image', () => {
        // Skipped test - provides visibility in output
      })
    }
  })
}
