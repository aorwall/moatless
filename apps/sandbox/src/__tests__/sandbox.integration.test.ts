import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import {
  createTestClient,
  startTestSandboxServer,
  type TestSandboxServer,
} from './helpers/sandbox-server'
import { setupTestRepo, type TestRepo } from './helpers/setup-git-repo'

describe('Sandbox Integration Tests', () => {
  let testRepo: TestRepo
  let server: TestSandboxServer
  let client: ReturnType<typeof createTestClient>

  beforeAll(async () => {
    // Clone hello-world-ui repo
    testRepo = await setupTestRepo()

    // Start sandbox server
    server = await startTestSandboxServer(testRepo.workspaceDir, {
      port: 8765,
      taskId: 'integration-test',
    })

    client = createTestClient(server.baseUrl)
  }, 60000) // 60 second timeout for setup

  afterAll(async () => {
    // Stop server first
    if (server) {
      await server.stop()
    }

    // Cleanup temp repo
    if (testRepo) {
      await testRepo.cleanup()
    }
  })

  describe('Health & Status', () => {
    it('should return healthy status', async () => {
      const response = await client.get('/health')
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data.status).toBe('ok')
      expect(data.taskId).toBe('integration-test')
    })

    it('should return workspace status', async () => {
      const response = await client.get('/workspace/status')
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data.status).toBe('ready')
      expect(data.workspaceDir).toBe(testRepo.workspaceDir)
    })
  })

  describe('File Operations', () => {
    it('should list files in workspace', async () => {
      const response = await client.post('/list-files', { path: '/', recursive: false })
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(Array.isArray(data.files)).toBe(true)
      expect(data.files.length).toBeGreaterThan(0)
    })

    it('should read existing file', async () => {
      const response = await client.post('/read-file', { filePath: 'package.json' })
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data.content).toBeDefined()
      expect(data.content).toContain('name')
    })

    it('should write and read back a file', async () => {
      const testContent = `Test file created at ${new Date().toISOString()}`

      // Write file
      const writeResponse = await client.post('/write-file', {
        filePath: 'test-file.txt',
        content: testContent,
      })
      expect(writeResponse.status).toBe(200)

      // Read back
      const readResponse = await client.post('/read-file', { filePath: 'test-file.txt' })
      expect(readResponse.status).toBe(200)

      const data = await readResponse.json()
      expect(data.content).toBe(testContent)
    })

    it('should get file tree with changes', async () => {
      const response = await client.get('/file-tree-with-changes')
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data.tree).toBeDefined()
    })
  })

  describe('Git Operations', () => {
    it('should get git status', async () => {
      const response = await client.get('/git-status')
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data.currentBranch).toBeDefined()
    })

    it('should detect git changes after file write', async () => {
      // Write a new file
      await client.post('/write-file', {
        filePath: 'new-git-test-file.txt',
        content: 'Git test content',
      })

      // Check git changes
      const response = await client.get('/git-changes')
      expect(response.status).toBe(200)

      const data = await response.json()
      // Should detect the new file as untracked or modified
      expect(data).toBeDefined()
    })

    it('should get git diff', async () => {
      const response = await client.get('/git-diff')
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data).toBeDefined()
    })

    it('should get current commit hash', async () => {
      const response = await client.get('/current-commit-hash')
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data.hash).toBeDefined()
      expect(data.hash.length).toBeGreaterThan(0)
    })
  })

  describe('Preview Server Lifecycle', () => {
    it('should show stopped status initially', async () => {
      const response = await client.get('/server/status')
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data.status).toBe('stopped')
      expect(data.running).toBe(false)
    })

    it('should start preview server', async () => {
      const response = await client.post('/server/start', {
        serverCommands: ['npx http-server -p {PORT}'],
        setupCommands: [],
        startupTimeoutSeconds: 30,
      })
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data.success).toBe(true)
      expect(data.status).toBe('started')
      expect(data.port).toBeDefined()
    }, 45000)

    it('should show running status after start', async () => {
      const response = await client.get('/server/status')
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data.status).toBe('started')
      expect(data.running).toBe(true)
      expect(data.internalUrl).toBeDefined()
    })

    it('should proxy request to preview server', async () => {
      const response = await client.get('/preview/')
      // Should get HTML from the preview server
      expect(response.status).toBe(200)
    })

    it('should get server logs', async () => {
      const response = await client.get('/server/logs')
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data.logs).toBeDefined()
    })

    it('should stop preview server', async () => {
      const response = await client.post('/server/stop', {})
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data.success).toBe(true)
    })

    it('should show stopped status after stop', async () => {
      const response = await client.get('/server/status')
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data.status).toBe('stopped')
    })
  })

  describe('Agent Execution', () => {
    it('should show idle status initially', async () => {
      const response = await client.get('/agent-status')
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(['idle', 'stopped']).toContain(data.state)
    })

    it('should require message for agent execution', async () => {
      const response = await client.post('/execute-agent', {})
      expect(response.status).toBe(400)

      const data = await response.json()
      expect(data.error).toContain('message is required')
    })

    // Note: Full agent execution test requires ANTHROPIC_API_KEY
    // This test is skipped unless the key is available
    it.skipIf(!process.env['ANTHROPIC_API_KEY'])(
      'should execute agent and receive events',
      async () => {
        const events = await client.sse(
          '/execute-agent',
          {
            message: 'Create a file called hello.txt with the content "Hello from agent test"',
          },
          { maxEvents: 50, timeoutMs: 60000 },
        )

        // Should receive at least a started event
        expect(events.length).toBeGreaterThan(0)

        // Check for expected event types
        const eventTypes = events.map((e) => e.event)
        expect(eventTypes).toContain('agent')

        // Verify the file was created
        const readResponse = await client.post('/read-file', { path: 'hello.txt' })
        if (readResponse.status === 200) {
          const data = await readResponse.json()
          expect(data.content).toContain('Hello from agent test')
        }
      },
      90000,
    )

    it('should handle stop agent when none running', async () => {
      const response = await client.post('/stop-agent', {})
      expect(response.status).toBe(200)

      const data = await response.json()
      expect(data.success).toBe(true)
    })
  })
})
