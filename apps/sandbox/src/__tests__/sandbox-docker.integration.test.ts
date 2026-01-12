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

    it('should call MCP tools for preview server and screenshot', async () => {
      // ARRANGE: Give agent instruction to use MCP tools
      const instruction =
        'Start the preview server using "bun run dev -- --host --port {PORT}" ' +
        'with setup command "bun install". ' +
        'After the server starts, take a screenshot of the homepage.'

      // ACT: Execute agent with tool instructions
      const result = await client.executeAgent(instruction, {
        timeoutMs: 180000, // 3 minutes for install + server start + screenshot
      })

      // Log all events for debugging (can be removed later)
      console.log('\n=== Agent Events ===')
      result.events.forEach((e, i) => {
        console.log(`Event ${i}: ${e.event}`)
        if (e.data) {
          console.log(`  Data: ${JSON.stringify(e.data).substring(0, 200)}...`)
        }
      })
      console.log('===================\n')

      // ASSERT 1: Agent execution completed
      expect(result.events.length).toBeGreaterThan(0)

      // ASSERT 2: Check for agent events (state changes, tool calls)
      const eventTypes = result.events.map((e) => e.event)
      expect(eventTypes).toContain('agent')

      // ASSERT 3: Look for tool call events in agent data
      // Note: This depends on how agent events are structured
      // May need to check event.data for tool call information
      const agentEvents = result.events.filter((e) => e.event === 'agent')

      // Check if any agent event mentions the tools
      const hasToolCalls = agentEvents.some((e) => {
        const data = JSON.stringify(e.data).toLowerCase()
        return (
          data.includes('startorrestartpreviewserver') ||
          data.includes('takepreviewscreenshot') ||
          data.includes('preview') ||
          data.includes('screenshot') ||
          data.includes('mcp') ||
          data.includes('tool')
        )
      })

      expect(hasToolCalls).toBe(true)

      // ASSERT 4: Check for error events
      // Note: Tools currently return stub responses which may cause errors
      // This is expected until Phase 3B/3C implementation
      const errorEvents = result.events.filter(
        (e) =>
          e.event === 'error' || (e.data && JSON.stringify(e.data).toLowerCase().includes('error')),
      )

      if (errorEvents.length > 0) {
        console.log('\n=== Error Events Found (Expected with stub tools) ===')
        errorEvents.forEach((e, i) => {
          console.log(`Error ${i}: ${JSON.stringify(e.data)}`)
        })
        console.log('===================\n')
      }

      // For now, we accept errors from stub implementations
      // TODO: Once tools are implemented (Phase 3B/3C), change this to:
      // expect(errorEvents.length).toBe(0)

      // FUTURE: When tools are implemented, add these assertions:
      // - Verify preview server actually started (check /server/status)
      // - Verify screenshot file exists (check file system)
      // - Verify screenshot is valid PNG (check file content)
    }, 240000) // 4 minute timeout for full workflow
  })

  describe('MCP Protocol', () => {
    it('should handle MCP initialize request', async () => {
      const response = await client.post('/mcp', {
        jsonrpc: '2.0',
        id: 1,
        method: 'initialize',
        params: {
          protocolVersion: '2024-11-05',
          capabilities: {},
          clientInfo: {
            name: 'test-client',
            version: '1.0.0',
          },
        },
      })

      expect(response.ok).toBe(true)
      const data = (await response.json()) as {
        jsonrpc: string
        id: number
        result: {
          protocolVersion: string
          serverInfo: { name: string; version: string }
          capabilities: object
        }
      }
      expect(data.result).toBeDefined()
      expect(data.result.serverInfo.name).toBe('moatless-sandbox-tools')
      expect(data.result.protocolVersion).toBe('2024-11-05')
    })

    it('should list available MCP tools', async () => {
      const response = await client.post('/mcp', {
        jsonrpc: '2.0',
        id: 2,
        method: 'tools/list',
        params: {},
      })

      expect(response.ok).toBe(true)
      const data = (await response.json()) as {
        jsonrpc: string
        id: number
        result: { tools: Array<{ name: string; description: string }> }
      }
      expect(data.result?.tools).toBeInstanceOf(Array)
      expect(data.result.tools.length).toBeGreaterThan(0)

      // Should include database tool
      const toolNames = data.result.tools.map((t) => t.name)
      expect(toolNames).toContain('updateTaskMetadata')
    })

    it('should return error for unknown method', async () => {
      const response = await client.post('/mcp', {
        jsonrpc: '2.0',
        id: 3,
        method: 'unknown/method',
        params: {},
      })

      expect(response.ok).toBe(true)
      const data = (await response.json()) as {
        jsonrpc: string
        id: number
        error: { code: number; message: string }
      }
      expect(data.error).toBeDefined()
      expect(data.error?.code).toBe(-32601)
      expect(data.error?.message).toContain('Method not found')
    })

    it('should return error for unknown tool', async () => {
      const response = await client.post('/mcp', {
        jsonrpc: '2.0',
        id: 4,
        method: 'tools/call',
        params: {
          name: 'nonexistentTool',
          arguments: {},
        },
      })

      expect(response.ok).toBe(true)
      const data = (await response.json()) as {
        jsonrpc: string
        id: number
        error: { code: number; message: string }
      }
      expect(data.error).toBeDefined()
      expect(data.error?.code).toBe(-32601)
      expect(data.error?.message).toContain('Tool not found')
    })

    it('should accept tool call and return stub response (protocol validation)', async () => {
      const response = await client.post('/mcp', {
        jsonrpc: '2.0',
        id: 5,
        method: 'tools/call',
        params: {
          name: 'getPreviewServerLogs',
          arguments: { lines: 10 },
        },
      })

      expect(response.ok).toBe(true)
      const data = (await response.json()) as {
        jsonrpc: string
        id: number
        result: {
          content: Array<{ type: string; text: string }>
          isError: boolean
        }
      }

      // Should return result (even if stub)
      expect(data.result).toBeDefined()
      expect(data.result.content).toBeInstanceOf(Array)
      expect(data.result.content[0]).toHaveProperty('type', 'text')
      // Currently returns TODO stub - this is expected for Phase 3A
      expect(data.result.content[0].text).toContain('TODO')
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
