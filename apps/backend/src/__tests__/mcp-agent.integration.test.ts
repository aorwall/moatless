import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { CreateTaskRequest, Event } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { EventService } from '../services/event.service.js'
import { build } from '../test/helper.js'
import { getDataDir } from '../utils/data-dir.js'
import { logger } from '../utils/logger.js'
import { createTestRepository, getTestEnvironmentConfig } from './utils/test-helpers.js'

/**
 * MCP Agent Integration Test
 *
 * This test specifically validates that an agent can successfully use the MCP updateTaskMetadata tool
 * to change task name and description. It focuses only on the MCP functionality without other workflow checks.
 */

describe('MCP Agent Integration Test', () => {
  let app: any
  let server: any
  let serverUrl: string
  let tempDir: string
  let createdRepositoryId: string
  let createdTaskId: string
  let eventService: EventService
  let eventSubscription: (() => void) | null = null
  let collectedEvents: Event[] = []

  beforeEach(async () => {
    // Create a temporary directory for our test git repository
    const dataDir = getDataDir()
    tempDir = await fs.mkdtemp(path.join(dataDir, 'mcp-agent-test-'))

    // Skip reading Claude events from disk for tests
    process.env['SKIP_CLAUDE_DISK_EVENTS'] = 'true'

    app = await build()

    // Start the server on a random available port
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
    collectedEvents = []
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

    delete process.env['SKIP_CLAUDE_DISK_EVENTS']
    delete process.env['TEST_SERVER_URL']

    // Close the server
    if (app) {
      await app.close()
    }
  })

  it('should run agent that uses MCP updateTaskMetadata tool to change task name and description', async () => {
    const envConfig = getTestEnvironmentConfig()
    logger.info('Running MCP agent test', { workspaceType: envConfig.workspaceType })

    // Step 1: Create repository
    const repository = await createTestRepository(app, tempDir, {
      setupCommands: ['echo "MCP agent test repository"'],
    })

    createdRepositoryId = repository.id
    logger.info('Test repository created', {
      repositoryId: repository.id,
      workspaceType: envConfig.workspaceType,
    })

    // Step 2: Create and start a task with explicit MCP tool usage instruction
    const createTaskRequest: CreateTaskRequest = {
      agentType: 'claude-code',
      repositoryId: repository.id,
      message:
        'Please use the updateTaskMetadata MCP tool to update this task. Change the task name to "agent-mcp-test" and the description to "Updated by agent via MCP". This is a test of MCP functionality.',
      start: true, // Start the agent immediately
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

    logger.info('Task created and agent starting', {
      taskId: createdTaskId,
      status: taskCreationResult.status,
    })

    // Step 3: Wait for agent to complete by monitoring events
    let agentStarted = false
    let agentCompleted = false
    let agentResult: string | null = null

    const maxWaitTime = 120000 // 120 seconds
    const startTime = Date.now()

    const agentPromise = new Promise<void>((resolve, reject) => {
      // Subscribe to events for this specific task
      eventSubscription = eventService.subscribe(createdTaskId, (event: Event) => {
        collectedEvents.push(event)

        logger.debug('Received event', {
          type: event.type,
          subType: event.subType,
          isError: event.isError,
        })

        // Check for agent started
        if (event.type === 'agent' && event.subType === 'started') {
          agentStarted = true
          logger.info('Agent started - MCP should be available')
        }

        // Check for agent completion
        if (event.type === 'agent' && event.subType === 'completed' && !event.isError) {
          agentCompleted = true
          agentResult = (event.payload?.result as string) || null
          logger.info('Agent completed successfully')
          resolve()
        }

        // Check for errors
        if (event.isError && event.type === 'agent') {
          console.error('Agent error event:', event.payload)
          reject(new Error(`Agent error: ${event.payload?.error || 'Unknown error'}`))
        }
      })

      // Set timeout
      setTimeout(() => {
        if (!agentCompleted) {
          reject(new Error(`Agent did not complete within ${maxWaitTime}ms`))
        }
      }, maxWaitTime)
    })

    // Wait for agent to complete
    await agentPromise

    // Step 4: Verify agent completed successfully
    expect(agentStarted).toBe(true)
    expect(agentCompleted).toBe(true)
    expect(agentResult).toBeTruthy()
    expect(typeof agentResult).toBe('string')

    // Step 5: Verify task metadata was actually updated via MCP
    logger.info('Verifying task metadata was updated by agent via MCP')
    const updatedTaskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })

    expect(updatedTaskResponse.statusCode).toBe(200)
    const updatedTask = JSON.parse(updatedTaskResponse.body)

    // Verify the specific values we asked the agent to set via MCP
    expect(updatedTask.name).toBe('agent-mcp-test')
    expect(updatedTask.description).toBe('Updated by agent via MCP')
    logger.info('✅ Task metadata successfully updated by agent via MCP tool', {
      name: updatedTask.name,
      description: updatedTask.description,
    })

    // Step 6: Verify agent mentioned using MCP in its response
    const lowerResult = agentResult!.toLowerCase()
    const mentionedMCPUsage =
      lowerResult.includes('updatetaskmetadata') ||
      lowerResult.includes('mcp') ||
      lowerResult.includes('updated') ||
      lowerResult.includes('agent-mcp-test') ||
      lowerResult.includes('updated by agent via mcp')

    expect(mentionedMCPUsage).toBe(true)
    logger.info('Agent mentioned MCP tool usage in response', { mentionedMCPUsage })

    console.log('✅ MCP agent integration test completed:', {
      agentStarted,
      agentCompleted,
      agentUsedMCP: mentionedMCPUsage,
      taskNameUpdated: updatedTask.name === 'agent-mcp-test',
      taskDescriptionUpdated: updatedTask.description === 'Updated by agent via MCP',
      finalName: updatedTask.name,
      finalDescription: updatedTask.description,
      executionTimeMs: Date.now() - startTime,
    })
  }, 180000) // 3 minute timeout for full agent execution
})

async function setupTempGitRepo(tempDir: string): Promise<void> {
  const { exec } = await import('node:child_process')
  const { promisify } = await import('node:util')
  const execAsync = promisify(exec)

  // Initialize git repository
  await execAsync('git init', { cwd: tempDir })
  await execAsync('git config user.name "MCP Agent Test"', { cwd: tempDir })
  await execAsync('git config user.email "mcp-agent@test.com"', { cwd: tempDir })

  // Create initial README
  const readmePath = path.join(tempDir, 'README.md')
  await fs.writeFile(
    readmePath,
    '# MCP Agent Test Repository\n\nRepository for testing MCP agent integration.\n',
  )

  // Add and commit initial file
  await execAsync('git add README.md', { cwd: tempDir })
  await execAsync('git commit -m "Initial commit for MCP agent test"', { cwd: tempDir })
}
