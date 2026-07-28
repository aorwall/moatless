/**
 * Docker integration tests for the Rust Sandbox gRPC service.
 *
 * These tests run the sandbox inside a Docker container and verify
 * the gRPC SandboxService endpoints work correctly, alongside the
 * HTTP health probe.
 *
 * Prerequisites:
 * - Docker daemon must be running
 * - Sandbox image: aorwall/moatless-vibe-sandbox-js:latest
 *   (or set SANDBOX_DOCKER_IMAGE env var)
 * - CLAUDE_CODE_OAUTH_TOKEN or ANTHROPIC_API_KEY for agent tests
 *
 * Run:
 *   task test:sandbox:docker
 *
 * This project stands on its own — its own deps, its own vitest config — because
 * it is the only suite that talks to a sandbox container directly instead of
 * through the backend. Every test self-skips when Docker or the image is absent.
 */
import * as grpc from '@grpc/grpc-js'
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest'
import {
  type DockerSandboxContext,
  dockerExec,
  hasAnthropicKey,
  isDockerAvailable,
  isPlatformCompatible,
  setupDockerSandbox,
  startDockerSandbox,
  stopDockerSandbox,
} from './helpers/docker-sandbox'
import {
  createGrpcSandboxClient,
  type GrpcSandboxClient,
  waitForGrpcReady,
} from './helpers/grpc-sandbox-client'

const dockerAvailable = await isDockerAvailable()
const platformCompatible = isPlatformCompatible()
const canRunTests = dockerAvailable && platformCompatible

describe.skipIf(!canRunTests)('Sandbox Docker gRPC Integration', () => {
  let ctx: DockerSandboxContext
  let grpcClient: GrpcSandboxClient

  beforeAll(async () => {
    ctx = await setupDockerSandbox()
    await startDockerSandbox(ctx)
    grpcClient = createGrpcSandboxClient(ctx.grpcPort)
    // HTTP health passed in startDockerSandbox; verify gRPC is also ready
    await waitForGrpcReady(grpcClient, 30_000)
  }, 120_000)

  afterAll(async () => {
    if (grpcClient) {
      grpcClient.close()
    }
    if (ctx) {
      await stopDockerSandbox(ctx)
    }
  }, 30_000)

  // ── Health & Status ─────────────────────────────────────────

  describe('Health & Status', () => {
    it('should return OK from HTTP /health', async () => {
      const response = await fetch(`http://localhost:${ctx.hostPort}/health`, {
        signal: AbortSignal.timeout(5000),
      })
      expect(response.ok).toBe(true)
      const text = await response.text()
      expect(text).toBe('OK')
    })

    it('should return healthy from gRPC CheckHealth', async () => {
      const resp = await grpcClient.checkHealth()
      expect(resp.healthy).toBe(true)
    })

    it('should return valid status JSON from gRPC GetStatus', async () => {
      const resp = await grpcClient.getStatus()
      expect(resp.statusJson).toBeTruthy()

      const status = JSON.parse(resp.statusJson)
      expect(status).toHaveProperty('taskId')
      expect(status).toHaveProperty('agent')
      expect(status.agent).toHaveProperty('status')
      expect(status).toHaveProperty('servers')
    })
  })

  // ── File Search ─────────────────────────────────────────────

  describe('File Search', () => {
    it('should find package.json with glob query', async () => {
      const resp = await grpcClient.searchFiles('**/package.json')
      expect(resp.files).toBeDefined()
      expect(Array.isArray(resp.files)).toBe(true)
      expect(resp.files.length).toBeGreaterThan(0)
      expect(resp.files.some((f) => f.includes('package.json'))).toBe(true)
    })

    it('should return empty list for non-matching query', async () => {
      const resp = await grpcClient.searchFiles('**/nonexistent-file-abc123.xyz')
      expect(resp.files).toBeDefined()
      expect(resp.files.length).toBe(0)
    })
  })

  // ── Server Management ───────────────────────────────────────

  describe('Server Management', () => {
    it('should return NOT_FOUND for nonexistent server', async () => {
      try {
        await grpcClient.getServerStatus('nonexistent')
        expect.fail('Expected gRPC NOT_FOUND error')
      } catch (err) {
        const grpcErr = err as grpc.ServiceError
        expect(grpcErr.code).toBe(grpc.status.NOT_FOUND)
      }
    })

    it('should return NOT_FOUND for StreamServerLogs on nonexistent server', async () => {
      try {
        await grpcClient.streamServerLogs('nonexistent')
        expect.fail('Expected gRPC NOT_FOUND error')
      } catch (err) {
        const grpcErr = err as grpc.ServiceError
        expect(grpcErr.code).toBe(grpc.status.NOT_FOUND)
      }
    })
  })

  // ── NotifyMessages ──────────────────────────────────────────

  describe('NotifyMessages', () => {
    it('should succeed without error (no-op wake)', async () => {
      const resp = await grpcClient.notifyMessages()
      expect(resp).toBeDefined()
    })
  })

  // ── StopAgent (no agent running) ────────────────────────────

  describe('StopAgent', () => {
    it('should return FAILED_PRECONDITION when no agent is running', async () => {
      try {
        await grpcClient.stopAgent()
        expect.fail('Expected gRPC FAILED_PRECONDITION error')
      } catch (err) {
        const grpcErr = err as grpc.ServiceError
        expect(grpcErr.code).toBe(grpc.status.FAILED_PRECONDITION)
      }
    })
  })

  // ── Agent Execution (requires Claude API key) ───────────────

  describe.skipIf(!hasAnthropicKey())('Agent Execution', () => {
    /**
     * Helper: wait for a file to appear inside the container.
     * The SDK message stream may not close promptly after the agent
     * finishes work, so polling for the file is more reliable than
     * waiting for the agent status to transition from "running".
     */
    async function waitForFile(filePath: string, timeoutMs = 120_000): Promise<boolean> {
      const deadline = Date.now() + timeoutMs
      while (Date.now() < deadline) {
        try {
          dockerExec(ctx.containerName, `test -f "${filePath}"`, { timeout: 5000 })
          return true
        } catch {
          // File not yet created
        }
        await new Promise((resolve) => setTimeout(resolve, 2000))
      }
      return false
    }

    /** Force agent into non-running state via StopAgent. */
    async function forceStopAgent(): Promise<void> {
      try {
        await grpcClient.stopAgent()
      } catch {
        // Already idle / no agent running — ignore
      }
      // Give the executor a moment to settle
      await new Promise((resolve) => setTimeout(resolve, 1000))
    }

    afterEach(async () => {
      // Ensure agent is stopped between tests so the next can start fresh
      await forceStopAgent()
    })

    it('should execute agent and create a file', async () => {
      const content = [
        {
          text: {
            text: 'Create a file called hello-from-grpc.txt with the content "Hello from gRPC agent test!"',
          },
        },
      ]
      await grpcClient.executeAgent(content)

      // Wait for the file to appear (proves agent ran successfully)
      const fileExists = await waitForFile(`${ctx.tempDir}/hello-from-grpc.txt`, 120_000)
      expect(fileExists).toBe(true)

      const fileContent = dockerExec(ctx.containerName, `cat ${ctx.tempDir}/hello-from-grpc.txt`)
      expect(fileContent).toContain('Hello from gRPC agent test')
    }, 180_000)

    it('should return ALREADY_EXISTS if agent is started twice', async () => {
      // Start agent
      const content = [{ text: { text: 'Create a file called second.txt with content "second"' } }]
      await grpcClient.executeAgent(content)

      // Immediately try to start another — should fail
      try {
        await grpcClient.executeAgent(content)
        expect.fail('Expected gRPC ALREADY_EXISTS error')
      } catch (err) {
        const grpcErr = err as grpc.ServiceError
        expect(grpcErr.code).toBe(grpc.status.ALREADY_EXISTS)
      }
    }, 180_000)
  })
})

// Provide skip message when tests cannot run
if (!canRunTests) {
  describe('Sandbox Docker gRPC Integration', () => {
    if (!dockerAvailable) {
      it.skip('Tests skipped - Docker not available', () => {})
    } else if (!platformCompatible) {
      it.skip('Tests skipped - Platform not compatible with sandbox image', () => {})
    }
  })
}
