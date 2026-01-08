import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { logger } from '../utils/logger'
import { E2EClient } from './utils/e2e-client'

/**
 * K8s Worker Pod E2E Integration Test
 *
 * This test validates the complete workflow for tasks that run on Kubernetes worker pods.
 * It tests against a deployed API using API key authentication.
 *
 * Requirements:
 * - E2E_API_URL: URL of the deployed API (e.g., https://moatless.xyz)
 * - E2E_API_KEY: API key for authentication (mvk_xxx format)
 *
 * The test covers:
 * 1. Repository creation
 * 2. Task creation with worker pod execution
 * 3. Polling for completion (includes K8s pod startup time)
 * 4. Result verification
 * 5. Cleanup
 *
 * Run with:
 * E2E_API_URL=https://moatless.xyz E2E_API_KEY=mvk_xxx bun test k8s-worker-e2e
 */
describe('K8s Worker Pod E2E Test', () => {
  const API_BASE_URL = process.env['E2E_API_URL']
  const API_KEY = process.env['E2E_API_KEY']

  let client: E2EClient
  let createdRepositoryId: string | null = null
  let createdTaskId: string | null = null

  beforeAll(async () => {
    if (!(API_BASE_URL && API_KEY)) {
      logger.warn(
        'Skipping K8s worker E2E test: E2E_API_URL and E2E_API_KEY environment variables are required',
      )
      return
    }

    client = new E2EClient(API_BASE_URL, API_KEY)

    // Verify API connectivity
    const isHealthy = await client.healthCheck()
    if (!isHealthy) {
      throw new Error(`Cannot connect to API at ${API_BASE_URL}`)
    }

    logger.info({ apiUrl: API_BASE_URL }, 'E2E client initialized')
  })

  afterAll(async () => {
    // Cleanup: delete created resources
    if (createdTaskId) {
      try {
        await client.deleteTask(createdTaskId)
        logger.info({ taskId: createdTaskId }, 'Deleted test task')
      } catch (error) {
        logger.warn({ error, taskId: createdTaskId }, 'Failed to delete test task')
      }
    }

    if (createdRepositoryId) {
      try {
        await client.deleteRepository(createdRepositoryId)
        logger.info({ repositoryId: createdRepositoryId }, 'Deleted test repository')
      } catch (error) {
        logger.warn(
          { error, repositoryId: createdRepositoryId },
          'Failed to delete test repository',
        )
      }
    }
  })

  it('should complete full workflow with K8s worker pod', async () => {
    // Skip if environment variables not set
    if (!(API_BASE_URL && API_KEY)) {
      console.log('Skipping: E2E_API_URL and E2E_API_KEY required')
      return
    }

    // 1. Create a test repository
    // Using a simple public repository for testing
    const repo = await client.createRepository({
      name: `e2e-test-${Date.now()}`,
      gitCloneUrl: 'https://github.com/aorwall/hello-world-ui.git',
      branch: 'main',
    })

    createdRepositoryId = repo.id
    logger.info({ repositoryId: repo.id, name: repo.name }, 'Created test repository')

    expect(repo.id).toBeDefined()
    expect(repo.name).toContain('e2e-test-')

    // 2. Create a task and start execution
    // This will trigger the K8s worker pod to spin up
    const taskResult = await client.createTask({
      agentType: 'claude-code',
      repositoryId: repo.id,
      message: 'List the files in the current directory and summarize what you see.',
      start: true,
    })

    createdTaskId = taskResult.taskId
    logger.info({ taskId: taskResult.taskId }, 'Created and started test task')

    expect(taskResult.taskId).toBeDefined()

    // 3. Wait for task completion
    // This includes K8s pod startup time (~30-60s) plus task execution
    const completedTask = await client.waitForTaskCompletion(taskResult.taskId, {
      timeout: 300000, // 5 minutes to account for pod startup
      pollInterval: 10000, // Poll every 10 seconds
    })

    logger.info(
      {
        taskId: completedTask.taskId,
        status: completedTask.status,
        iterationsCount: completedTask.iterations.length,
      },
      'Task completed',
    )

    // 4. Verify results
    // Task terminal states: 'error' | 'merged' | 'closed'
    // Iteration terminal states: 'completed' | 'failed'
    expect(['error', 'merged', 'closed']).toContain(completedTask.status)
    expect(completedTask.iterations.length).toBeGreaterThan(0)

    // Check the last iteration completed successfully
    const lastIteration = completedTask.iterations[completedTask.iterations.length - 1]
    expect(lastIteration?.status).toBe('completed')

    logger.info('K8s worker pod E2E test passed successfully')
  }, 360000) // Test timeout: 6 minutes (includes pod startup + execution)

  it('should handle task with file creation', async () => {
    // Skip if environment variables not set
    if (!(API_BASE_URL && API_KEY)) {
      console.log('Skipping: E2E_API_URL and E2E_API_KEY required')
      return
    }

    // Skip if no repository was created in the previous test
    if (!createdRepositoryId) {
      // Create a new repository for this test
      const repo = await client.createRepository({
        name: `e2e-test-file-${Date.now()}`,
        gitCloneUrl: 'https://github.com/aorwall/hello-world-ui.git',
        branch: 'main',
      })
      createdRepositoryId = repo.id
    }

    // Create a task that creates a file
    const taskResult = await client.createTask({
      agentType: 'claude-code',
      repositoryId: createdRepositoryId,
      message: 'Create a file called hello.txt with the content "Hello from E2E test"',
      start: true,
    })

    createdTaskId = taskResult.taskId
    logger.info({ taskId: taskResult.taskId }, 'Created file creation task')

    // Wait for completion
    const completedTask = await client.waitForTaskCompletion(taskResult.taskId, {
      timeout: 300000,
      pollInterval: 10000,
    })

    // Verify the task completed
    expect(['error', 'merged', 'closed']).toContain(completedTask.status)

    // The last iteration should show some git changes if a file was created
    const lastIteration = completedTask.iterations[completedTask.iterations.length - 1]
    expect(lastIteration?.status).toBe('completed')

    logger.info('File creation E2E test passed successfully')
  }, 360000)
})
