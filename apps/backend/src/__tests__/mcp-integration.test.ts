import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { CreateTaskRequest, Event } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { EventService } from '../features/events/index.js'
import { build } from '../test/helper.js'
import { getDataDir } from '../utils/data-dir.js'
import { logger } from '../utils/logger.js'
import { createTestRepository, getTestEnvironmentConfig } from './utils/test-helpers.js'

/**
 * MCP (Model Context Protocol) Integration Test
 *
 * This test runs an actual agent and tests MCP functionality while the agent is running:
 * - Agent starts and creates persistent MCP server
 * - Agent reports available MCP tools
 * - Direct MCP endpoint testing while agent runs
 * - Tool invocation and verification
 */

describe('MCP Integration Test with Running Agent', () => {
  let app: any
  let server: any
  let serverUrl: string
  let tempDir: string
  let createdRepositoryId: string
  let createdTaskId: string
  let eventService: EventService
  let eventSubscription: (() => void) | null = null
  const _collectedEvents: Event[] = []

  beforeEach(async () => {
    // Create a temporary directory for our test git repository
    const dataDir = getDataDir()
    tempDir = await fs.mkdtemp(path.join(dataDir, 'mcp-integration-test-'))

    // Skip reading Claude events from disk for tests
    process.env['SKIP_CLAUDE_DISK_EVENTS'] = 'true'

    app = await build()

    // Start the server on a random available port for SSE testing
    // This is CRITICAL for MCP to work - it needs a real HTTP server
    await app.listen({ port: 0, host: '127.0.0.1' })

    // Get the actual server URL from the app's server property
    server = app.server
    const address = server?.address()
    const port = address?.port || 3000
    serverUrl = `http://127.0.0.1:${port}`

    // Set the test server URL for MCP server URL construction
    process.env['TEST_SERVER_URL'] = serverUrl

    logger.info('Test server started', { serverUrl })

    // Get event service from app services
    eventService = app.services.event
    if (!eventService) {
      throw new Error('EventService not available in test app')
    }

    // Initialize a git repository in the temp directory
    await setupTempGitRepo(tempDir)

    // Reset collected events
    _collectedEvents.length = 0
  })

  afterEach(async () => {
    // Unsubscribe from events if subscribed
    if (eventSubscription) {
      eventSubscription()
      eventSubscription = null
    }

    // Stop any running agents
    if (createdTaskId) {
      try {
        await app.inject({
          method: 'POST',
          url: `/api/v1/tasks/${createdTaskId}/stop`,
        })
        await new Promise((resolve) => setTimeout(resolve, 100))
      } catch (_e) {
        // Agent might not be running
      }
    }

    // Cleanup task
    if (createdTaskId) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/tasks/${createdTaskId}`,
        })
      } catch (_e) {
        // Task might already be deleted
      }
    }

    // Cleanup repository
    if (createdRepositoryId) {
      try {
        await app.inject({
          method: 'DELETE',
          url: `/api/v1/repositories/${createdRepositoryId}`,
        })
      } catch (_e) {
        // Repository might already be deleted
      }
    }

    // Clean up temp directory
    if (tempDir) {
      try {
        await fs.rm(tempDir, { recursive: true, force: true })
      } catch (_e) {
        // Directory might already be deleted
      }
    }

    Reflect.deleteProperty(process.env, 'SKIP_CLAUDE_DISK_EVENTS')
    Reflect.deleteProperty(process.env, 'TEST_SERVER_URL')

    // Close the server
    if (app) {
      await app.close()
    }
  })

  it('should test MCP endpoint functionality and update task metadata', async () => {
    const envConfig = getTestEnvironmentConfig()
    logger.info('Running MCP test', { workspaceType: envConfig.workspaceType })

    // Step 1: Create repository
    const repository = await createTestRepository(app, tempDir, {
      setupCommands: ['echo "MCP test repository"'],
    })

    createdRepositoryId = repository.id
    logger.info('Test repository created', {
      repositoryId: repository.id,
      workspaceType: envConfig.workspaceType,
    })

    // Step 2: Create a task
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
      message: 'Test task for MCP functionality',
      start: false,
    }

    const createTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks',
      payload: createTaskRequest,
    })

    expect(createTaskResponse.statusCode).toBe(200)
    const taskCreationResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskCreationResult.taskId
    expect(taskCreationResult.status).toBe('open')

    logger.info('Task created', {
      taskId: createdTaskId,
      status: taskCreationResult.status,
    })

    // Step 3: Test MCP endpoint functionality
    logger.info('Testing MCP endpoint functionality')
    const mcpUrl = `/api/v1/tasks/${createdTaskId}/mcp`

    // Test initialize request
    const initializeRequest = {
      jsonrpc: '2.0',
      method: 'initialize',
      id: 1,
    }

    const initializeResponse = await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: initializeRequest,
    })

    expect(initializeResponse.statusCode).toBe(200)
    const initResult = JSON.parse(initializeResponse.body)
    expect(initResult.jsonrpc).toBe('2.0')
    expect(initResult.id).toBe(1)
    expect(initResult.result.protocolVersion).toBe('2024-11-05')
    expect(initResult.result.serverInfo.name).toBe('moatless-tools')
    logger.info('✅ MCP initialize request successful')

    // Test tools/list request
    const toolsListRequest = {
      jsonrpc: '2.0',
      method: 'tools/list',
      id: 2,
    }

    const toolsListResponse = await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: toolsListRequest,
    })

    expect(toolsListResponse.statusCode).toBe(200)
    const toolsResult = JSON.parse(toolsListResponse.body)
    expect(toolsResult.jsonrpc).toBe('2.0')
    expect(toolsResult.id).toBe(2)
    expect(Array.isArray(toolsResult.result.tools)).toBe(true)

    const updateToolFound = toolsResult.result.tools.find(
      (tool: any) => tool.name === 'updateTaskMetadata',
    )
    expect(updateToolFound).toBeTruthy()
    expect(updateToolFound.description).toBe(
      'Update task metadata (name and description) for the current task',
    )
    logger.info('✅ MCP tools/list request successful, updateTaskMetadata tool found')

    // Step 4: Test tools/call to update task metadata
    const toolCallRequest = {
      jsonrpc: '2.0',
      method: 'tools/call',
      params: {
        name: 'updateTaskMetadata',
        arguments: {
          name: 'mcp-test-task',
          description: 'Task updated by MCP tool',
        },
      },
      id: 3,
    }

    const toolCallResponse = await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: toolCallRequest,
    })

    expect(toolCallResponse.statusCode).toBe(200)
    const toolCallResult = JSON.parse(toolCallResponse.body)
    expect(toolCallResult.jsonrpc).toBe('2.0')
    expect(toolCallResult.id).toBe(3)
    expect(toolCallResult.result.content).toBeTruthy()
    expect(toolCallResult.result.content[0].type).toBe('text')
    expect(toolCallResult.result.content[0].text).toContain('Successfully updated task metadata')
    logger.info('✅ MCP tools/call request successful')

    // Step 5: Verify task metadata was actually updated
    logger.info('Verifying task metadata was updated')
    const updatedTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })

    expect(updatedTaskResponse.statusCode).toBe(200)
    const updatedTask = JSON.parse(updatedTaskResponse.body)

    // Verify the specific values we set via MCP
    expect(updatedTask.name).toBe('mcp-test-task')
    expect(updatedTask.description).toBe('Task updated by MCP tool')
    logger.info('✅ Task metadata successfully updated via MCP tool', {
      name: updatedTask.name,
      description: updatedTask.description,
    })

    console.log('✅ MCP integration test completed:', {
      mcpEndpointTested: true,
      initializeWorked: true,
      toolsListWorked: true,
      toolsCallWorked: true,
      taskMetadataUpdated: true,
      finalName: updatedTask.name,
      finalDescription: updatedTask.description,
    })
  }, 30000) // 30 second timeout for direct MCP testing
})

async function setupTempGitRepo(tempDir: string): Promise<void> {
  const { exec } = await import('node:child_process')
  const { promisify } = await import('node:util')
  const execAsync = promisify(exec)

  // Initialize git repository
  await execAsync('git init', { cwd: tempDir })
  await execAsync('git config user.name "MCP Test"', { cwd: tempDir })
  await execAsync('git config user.email "mcp@test.com"', { cwd: tempDir })

  // Create initial README
  const readmePath = path.join(tempDir, 'README.md')
  await fs.writeFile(
    readmePath,
    '# MCP Test Repository\n\nRepository for testing MCP integration with running agent.\n',
  )

  // Add and commit initial file
  await execAsync('git add README.md', { cwd: tempDir })
  await execAsync('git commit -m "Initial commit for MCP test"', {
    cwd: tempDir,
  })
}
