import { promises as fs } from 'node:fs'
import path from 'node:path'
import type { CreateVibeTaskRequest, Event } from '@moatless/types'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { EventService } from '../services/event.service.js'
import { build } from '../test/helper.js'
import { getDataDir } from '../utils/data-dir.js'
import { logger } from '../utils/logger.js'
import { cleanupTestResources, getTestEnvironmentConfig } from './utils/test-helpers.js'

/**
 * MCP Preview Server Integration Test
 *
 * This test verifies that the MCP (Model Context Protocol) preview server management tools
 * work correctly with vibe tasks that have preview server configuration.
 *
 * The test covers:
 * - Creating a vibe task with a template that has preview server config
 * - Verifying MCP tools are available for preview server management
 * - Testing complete server lifecycle (start, status, logs, restart, stop)
 * - Error handling scenarios
 */

/**
 * Helper function to wait for workspace environment to be ready by checking task setup
 */
async function waitForWorkspaceReady(app: any, taskId: string, timeoutMs = 60000): Promise<void> {
  const startTime = Date.now()
  const pollInterval = 2000 // Poll every 2 seconds

  logger.info('Waiting for workspace to be ready', { taskId, timeoutMs })

  while (Date.now() - startTime < timeoutMs) {
    // Try to start the task to see if workspace is ready
    const startResponse = await app.inject({
      method: 'POST',
      url: `/api/v1/tasks/${taskId}/start`,
    })

    if (startResponse.statusCode === 200) {
      logger.info('✅ Workspace is ready - task can be started', {
        taskId,
        elapsed: Date.now() - startTime,
      })
      return
    }
    if (startResponse.statusCode === 400) {
      const errorBody = JSON.parse(startResponse.body)
      if (errorBody.message?.includes('workspace')) {
        logger.debug('Workspace not ready yet', {
          taskId,
          message: errorBody.message,
          elapsed: Date.now() - startTime,
        })
      } else {
        // Some other error, but workspace might be ready
        logger.info('✅ Workspace appears ready (non-workspace error)', {
          taskId,
          elapsed: Date.now() - startTime,
          error: errorBody.message,
        })
        return
      }
    } else {
      logger.debug('Workspace status check failed', {
        taskId,
        statusCode: startResponse.statusCode,
        body: startResponse.body,
        elapsed: Date.now() - startTime,
      })
    }

    await new Promise((resolve) => setTimeout(resolve, pollInterval))
  }

  throw new Error(`Workspace setup timed out after ${timeoutMs}ms`)
}

describe('MCP Preview Server Integration Test', () => {
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
    tempDir = await fs.mkdtemp(path.join(dataDir, 'mcp-preview-server-test-'))

    // Skip reading Claude events from disk for tests
    process.env.SKIP_CLAUDE_DISK_EVENTS = 'true'

    app = await build()

    // Start the server on a random available port for MCP testing
    // This is CRITICAL for MCP to work - it needs a real HTTP server
    await app.listen({ port: 0, host: '127.0.0.1' })

    // Get the actual server URL from the app's server property
    server = app.server
    const address = server?.address()
    const port = address?.port || 3000
    serverUrl = `http://127.0.0.1:${port}`

    // Set the test server URL for MCP server URL construction
    process.env.TEST_SERVER_URL = serverUrl

    logger.info('MCP Preview Server test server started', { serverUrl })

    // Get event service from app services
    eventService = app.services.event
    if (!eventService) {
      throw new Error('EventService not available in test app')
    }

    // Reset collected events
    _collectedEvents.length = 0
  })

  afterEach(async () => {
    // Unsubscribe from events if subscribed
    if (eventSubscription) {
      eventSubscription()
      eventSubscription = null
    }

    // Stop any running preview servers
    if (createdTaskId) {
      try {
        // Use MCP to stop the server first
        await app.inject({
          method: 'POST',
          url: `/api/v1/tasks/${createdTaskId}/mcp`,
          payload: {
            jsonrpc: '2.0',
            method: 'tools/call',
            params: {
              name: 'stopPreviewServer',
              arguments: {},
            },
            id: 'cleanup-stop',
          },
        })
        await new Promise((resolve) => setTimeout(resolve, 2000)) // Wait for server to stop
      } catch (_e) {
        // Server might not be running
      }
    }

    // Cleanup test resources
    await cleanupTestResources(app, {
      taskId: createdTaskId,
      repositoryId: createdRepositoryId,
      tempDir,
    })

    delete process.env.SKIP_CLAUDE_DISK_EVENTS
    delete process.env.TEST_SERVER_URL

    // Close the server
    if (app) {
      await app.close()
    }
  })

  it('should manage preview server through MCP tools for vibe task', async () => {
    const envConfig = getTestEnvironmentConfig()
    logger.info('Running MCP Preview Server test', { workspaceType: envConfig.workspaceType })

    // Step 1: Create a vibe task with hello-world-ui template (has preview server config)
    const createVibeTaskRequest: CreateVibeTaskRequest = {
      agentType: 'claude-code',
      templateId: 'hello-world-ui',
      message: 'Test vibe task for MCP preview server functionality',
      start: false,
    }

    const createTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks/vibe',
      payload: createVibeTaskRequest,
    })

    expect(createTaskResponse.statusCode).toBe(200)
    const taskCreationResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskCreationResult.taskId
    expect(createdTaskId).toBeTruthy()

    logger.info('Vibe task created for MCP preview server test', {
      taskId: createdTaskId,
      templateId: 'hello-world-ui',
    })

    // Get the repository ID from the task
    const taskResponse = await app.inject({
      method: 'GET',
      url: `/api/v1/tasks/${createdTaskId}`,
    })
    expect(taskResponse.statusCode).toBe(200)
    const task = JSON.parse(taskResponse.body)
    createdRepositoryId = task.repositoryId

    // Step 2: Wait for workspace environment to be ready
    logger.info('Waiting for workspace to be ready before testing MCP tools')
    await waitForWorkspaceReady(app, createdTaskId)

    // Step 3: Test MCP initialization and tools listing
    logger.info('Testing MCP initialization and tools listing')
    const mcpUrl = `/api/v1/tasks/${createdTaskId}/mcp`

    // Initialize MCP connection
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
    logger.info('✅ MCP initialize successful')

    // Get available tools
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

    // Verify preview server tools are available
    const tools = toolsResult.result.tools
    const toolNames = tools.map((tool: any) => tool.name)

    const expectedPreviewTools = [
      'startPreviewServer',
      'stopPreviewServer',
      'restartPreviewServer',
      'getPreviewServerStatus',
      'getPreviewServerLogs',
    ]

    for (const toolName of expectedPreviewTools) {
      expect(toolNames).toContain(toolName)
      logger.info(`✅ Found expected MCP tool: ${toolName}`)
    }

    // Step 4: Test server lifecycle management

    // 4a. Check initial server status (should not be running)
    logger.info('Testing initial server status')
    const initialStatusResponse = await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: {
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'getPreviewServerStatus',
          arguments: {},
        },
        id: 'status-1',
      },
    })

    expect(initialStatusResponse.statusCode).toBe(200)
    const initialStatus = JSON.parse(initialStatusResponse.body)
    expect(initialStatus.result.content[0].text).toContain('stopped')
    logger.info('✅ Initial server status: stopped')

    // 4b. Start the preview server
    logger.info('Testing server start')
    const startServerResponse = await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: {
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'startPreviewServer',
          arguments: {},
        },
        id: 'start-1',
      },
    })

    expect(startServerResponse.statusCode).toBe(200)
    const startResult = JSON.parse(startServerResponse.body)
    expect(startResult.result.content[0].text).toContain('started successfully')
    logger.info('✅ Server started successfully')

    // Wait a bit for server to fully start
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // 4c. Check server status after start
    logger.info('Testing server status after start')
    const runningStatusResponse = await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: {
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'getPreviewServerStatus',
          arguments: {},
        },
        id: 'status-2',
      },
    })

    expect(runningStatusResponse.statusCode).toBe(200)
    const runningStatus = JSON.parse(runningStatusResponse.body)
    const statusText = runningStatus.result.content[0].text
    expect(statusText).toContain('running')
    expect(statusText).toContain('URL:')
    logger.info('✅ Server status shows running with URL')

    // 4d. Get server logs
    logger.info('Testing server logs retrieval')
    const logsResponse = await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: {
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'getPreviewServerLogs',
          arguments: {
            lines: 50,
          },
        },
        id: 'logs-1',
      },
    })

    expect(logsResponse.statusCode).toBe(200)
    const logsResult = JSON.parse(logsResponse.body)
    const logContent = logsResult.result.content[0].text
    expect(logContent).toBeTruthy()
    expect(logContent).not.toBe('No logs available')
    logger.info('✅ Server logs retrieved successfully')

    // 4e. Restart the server
    logger.info('Testing server restart')
    const restartResponse = await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: {
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'restartPreviewServer',
          arguments: {},
        },
        id: 'restart-1',
      },
    })

    expect(restartResponse.statusCode).toBe(200)
    const restartResult = JSON.parse(restartResponse.body)
    expect(restartResult.result.content[0].text).toContain('restarted successfully')
    logger.info('✅ Server restarted successfully')

    // Wait a bit for server to restart
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // 4f. Stop the server
    logger.info('Testing server stop')
    const stopResponse = await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: {
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'stopPreviewServer',
          arguments: {},
        },
        id: 'stop-1',
      },
    })

    expect(stopResponse.statusCode).toBe(200)
    const stopResult = JSON.parse(stopResponse.body)
    expect(stopResult.result.content[0].text).toContain('stopped successfully')
    logger.info('✅ Server stopped successfully')

    // 4g. Verify server is stopped
    logger.info('Testing final server status')
    const finalStatusResponse = await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: {
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'getPreviewServerStatus',
          arguments: {},
        },
        id: 'status-3',
      },
    })

    expect(finalStatusResponse.statusCode).toBe(200)
    const finalStatus = JSON.parse(finalStatusResponse.body)
    expect(finalStatus.result.content[0].text).toContain('stopped')
    logger.info('✅ Final server status confirmed: stopped')

    console.log('✅ MCP Preview Server integration test completed successfully:', {
      mcpInitialized: true,
      previewToolsAvailable: true,
      serverLifecycleTested: true,
      startWorked: true,
      statusWorked: true,
      logsWorked: true,
      restartWorked: true,
      stopWorked: true,
    })
  }, 120000) // 2 minute timeout for comprehensive server testing

  it('should handle error cases properly', async () => {
    const envConfig = getTestEnvironmentConfig()
    logger.info('Running MCP Preview Server error handling test', {
      workspaceType: envConfig.workspaceType,
    })

    // Create a vibe task for error testing
    const createVibeTaskRequest: CreateVibeTaskRequest = {
      agentType: 'claude-code',
      templateId: 'hello-world-ui',
      message: 'Test task for error handling',
      start: false,
    }

    const createTaskResponse = await app.inject({
      method: 'POST',
      url: '/api/v1/tasks/vibe',
      payload: createVibeTaskRequest,
    })

    expect(createTaskResponse.statusCode).toBe(200)
    const taskCreationResult = JSON.parse(createTaskResponse.body)
    createdTaskId = taskCreationResult.taskId

    // Wait for workspace to be ready before testing MCP tools
    logger.info('Waiting for workspace to be ready before testing error cases')
    await waitForWorkspaceReady(app, createdTaskId)

    const mcpUrl = `/api/v1/tasks/${createdTaskId}/mcp`

    // Test stopping a server that's not running
    logger.info('Testing stop server when not running')
    const stopNotRunningResponse = await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: {
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'stopPreviewServer',
          arguments: {},
        },
        id: 'error-stop-1',
      },
    })

    expect(stopNotRunningResponse.statusCode).toBe(200)
    const stopNotRunningResult = JSON.parse(stopNotRunningResponse.body)
    // Should handle gracefully - either success (already stopped) or appropriate message
    expect(stopNotRunningResult.result.content[0].text).toBeTruthy()
    logger.info('✅ Stop server when not running handled properly')

    // Start server for additional error tests
    await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: {
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'startPreviewServer',
          arguments: {},
        },
        id: 'error-start-1',
      },
    })

    // Wait for server to start
    await new Promise((resolve) => setTimeout(resolve, 3000))

    // Test starting server when already running
    logger.info('Testing start server when already running')
    const startAlreadyRunningResponse = await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: {
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'startPreviewServer',
          arguments: {},
        },
        id: 'error-start-2',
      },
    })

    expect(startAlreadyRunningResponse.statusCode).toBe(200)
    const startAlreadyRunningResult = JSON.parse(startAlreadyRunningResponse.body)
    // Should handle gracefully - either success (already running) or appropriate message
    expect(startAlreadyRunningResult.result.content[0].text).toBeTruthy()
    logger.info('✅ Start server when already running handled properly')

    // Clean up - stop the server
    await app.inject({
      method: 'POST',
      url: mcpUrl,
      payload: {
        jsonrpc: '2.0',
        method: 'tools/call',
        params: {
          name: 'stopPreviewServer',
          arguments: {},
        },
        id: 'error-cleanup',
      },
    })

    console.log('✅ MCP Preview Server error handling test completed successfully')
  }, 60000) // 1 minute timeout for error testing
})
