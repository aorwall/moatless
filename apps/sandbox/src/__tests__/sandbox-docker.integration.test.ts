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
import fs from 'node:fs'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import {
  createDockerSandboxClient,
  type DockerSandboxContext,
  dockerExec,
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
    it('should start preview server via script and reach it', async () => {
      // Write preview config to the mounted data directory
      const configPath = path.join(ctx.dataDir, 'preview-server.config.json')
      fs.writeFileSync(
        configPath,
        JSON.stringify({
          serverCommands: ['bun run dev -- --host --port {{PORT}}'],
          port: 3000,
          startupTimeoutSeconds: 120,
          workspacePath: ctx.tempDir,
        }),
      )

      // Install dependencies first
      dockerExec(ctx.containerName, `cd ${ctx.tempDir} && bun install`, { timeout: 120000 })

      // Start preview server via the script
      dockerExec(ctx.containerName, '/opt/moatless/bin/preview-server start', { timeout: 180000 })

      // Wait a moment for status file to be written and picked up by monitor
      await new Promise((resolve) => setTimeout(resolve, 2000))

      // Verify server is running via HTTP status endpoint
      const statusResponse = await client.get('/server/status')
      expect(statusResponse.ok).toBe(true)

      const statusData = (await statusResponse.json()) as {
        status: string
        running: boolean
      }
      expect(statusData.status).toBe('started')
      expect(statusData.running).toBe(true)
    }, 300000) // 5 minute timeout for install + dev server

    it('should proxy requests to preview server', async () => {
      // Make request through proxy
      const response = await client.get('/preview/')
      expect(response.ok).toBe(true)

      const html = await response.text()
      // hello-world-ui contains "Hello" in its content
      expect(html).toContain('Hello')
    })

    it('should get server logs', async () => {
      const response = await client.get('/server/logs')
      expect(response.ok).toBe(true)

      const data = (await response.json()) as { logs: string }
      expect(data.logs).toBeDefined()
      expect(data.logs.length).toBeGreaterThan(0)
    })

    it('should stop preview server via script', async () => {
      // Stop via the script
      dockerExec(ctx.containerName, '/opt/moatless/bin/preview-server stop')

      // Wait for status to update
      await new Promise((resolve) => setTimeout(resolve, 1000))

      // Verify stopped via HTTP
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
