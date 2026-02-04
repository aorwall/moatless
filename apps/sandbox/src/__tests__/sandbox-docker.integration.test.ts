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

  describe('File Search', () => {
    it('should search files in workspace', async () => {
      const response = await client.post('/search-files', {
        query: 'package.json',
        path: '.',
      })
      expect(response.ok).toBe(true)

      const data = (await response.json()) as { files: unknown[] }
      expect(Array.isArray(data.files)).toBe(true)
      expect(data.files.length).toBeGreaterThan(0)
    })
  })

  describe('Preview Server', () => {
    it('should start preview server via script and reach it', async () => {
      // Write preview config to the mounted data directory
      // The generic server script uses <id>-server.config.json format
      const configPath = path.join(ctx.dataDir, 'preview-server.config.json')
      fs.writeFileSync(
        configPath,
        JSON.stringify({
          startCommands: ['PORT=3000 bun run dev -- --host'],
          port: 3000,
          startupTimeoutSeconds: 120,
          workspacePath: ctx.tempDir,
        }),
      )

      // Install dependencies first
      dockerExec(ctx.containerName, `cd ${ctx.tempDir} && bun install`, { timeout: 120000 })

      // Start preview server via the generic server script
      dockerExec(ctx.containerName, '/opt/moatless/bin/server preview start', { timeout: 180000 })

      // Poll for server to become ready (startup includes bun install + dev server boot)
      let statusData = { status: 'unknown', running: false }
      const deadline = Date.now() + 120_000
      while (Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, 2000))
        const statusResponse = await client.get('/server/status')
        if (statusResponse.ok) {
          statusData = (await statusResponse.json()) as { status: string; running: boolean }
          if (statusData.status === 'started' || statusData.status === 'failed') break
        }
      }
      expect(statusData.status).toBe('started')
      expect(statusData.running).toBe(true)
    }, 300000) // 5 minute timeout for install + dev server

    it('should get server logs', async () => {
      const response = await client.get('/server/logs')
      expect(response.ok).toBe(true)

      const data = (await response.json()) as { logs: string }
      expect(data.logs).toBeDefined()
      expect(data.logs.length).toBeGreaterThan(0)
    })

    it('should stop preview server via script', async () => {
      // Stop via the generic server script
      dockerExec(ctx.containerName, '/opt/moatless/bin/server preview stop')

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

      // Verify the file was created via docker exec (read-file endpoint was removed)
      const fileContent = dockerExec(ctx.containerName, `cat ${ctx.tempDir}/hello-from-agent.txt`)
      expect(fileContent).toContain('Hello from Claude agent test')
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
