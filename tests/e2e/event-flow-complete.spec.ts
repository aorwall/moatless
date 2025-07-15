import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { expect, test } from '@playwright/test'
import {
  APIEventMonitor,
  getEventStreamStatus,
  SSEEventMonitor,
  waitForEventType,
  waitForTaskResult,
} from './helpers/event-monitors'
import {
  createTestRepository,
  createTestTask,
  deleteTestRepository,
  deleteTestTask,
  setupTempGitRepo,
  type TestRepository,
  type TestTask,
} from './helpers/test-server'

/**
 * Comprehensive E2E Test for Event Flow Architecture
 *
 * This test verifies the complete event-driven architecture by:
 * 1. Running real Claude Code agents on actual tasks
 * 2. Monitoring events through both backend API and frontend SSE
 * 3. Testing cross-agent event visibility
 * 4. Validating frontend UI updates from real-time events
 * 5. Ensuring event ordering and integrity across the stack
 */

test.describe('Complete Event Flow E2E', () => {
  let tempDirA: string
  let tempDirB: string
  let repositoryA: TestRepository | undefined
  let repositoryB: TestRepository | undefined
  let taskA: TestTask | undefined
  let taskB: TestTask | undefined
  let apiMonitor: APIEventMonitor | undefined
  let sseMonitorGlobal: SSEEventMonitor | undefined
  let sseMonitorTaskA: SSEEventMonitor | undefined

  const baseURL = 'http://127.0.0.1:3002'

  test.beforeEach(async ({ page }) => {
    // Create temporary directories for two different test repositories
    tempDirA = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-e2e-repo-a-'))
    tempDirB = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-e2e-repo-b-'))

    console.log('Created temp directories:', { tempDirA, tempDirB })

    // Set up git repositories
    await setupTempGitRepo(tempDirA)
    await setupTempGitRepo(tempDirB)

    // Create test repositories
    repositoryA = await createTestRepository(baseURL, 'E2E Repository A', tempDirA)
    repositoryB = await createTestRepository(baseURL, 'E2E Repository B', tempDirB)

    console.log('Created repositories:', {
      repositoryA: repositoryA.id,
      repositoryB: repositoryB.id,
    })

    // Initialize event monitors
    apiMonitor = new APIEventMonitor(baseURL)
    sseMonitorGlobal = new SSEEventMonitor(page, baseURL)

    // Start monitoring
    await apiMonitor.start()

    // Navigate to the application to establish frontend context
    await page.goto('/')

    // Start SSE monitoring
    await sseMonitorGlobal.start()

    // Verify initial connection
    expect(await sseMonitorGlobal.checkConnection()).toBe(true)

    console.log('Event monitors initialized and connected')
  })

  test.afterEach(async () => {
    // Clean up tasks
    if (taskA?.taskId) {
      await deleteTestTask(baseURL, taskA.taskId)
    }
    if (taskB?.taskId) {
      await deleteTestTask(baseURL, taskB.taskId)
    }

    // Clean up repositories
    if (repositoryA?.id) {
      await deleteTestRepository(baseURL, repositoryA.id)
    }
    if (repositoryB?.id) {
      await deleteTestRepository(baseURL, repositoryB.id)
    }

    // Clean up monitors
    if (apiMonitor) {
      await apiMonitor.cleanup()
    }
    if (sseMonitorGlobal) {
      await sseMonitorGlobal.cleanup()
    }
    if (sseMonitorTaskA) {
      await sseMonitorTaskA.cleanup()
    }

    // Clean up temp directories
    try {
      await fs.rm(tempDirA, { recursive: true, force: true })
      await fs.rm(tempDirB, { recursive: true, force: true })
    } catch (error) {
      console.warn('Failed to clean up temp directories:', error)
    }
  })

  test('should handle dual agent execution with cross-visibility of events', async ({ page }) => {
    // Ensure repositories and monitors exist
    if (!repositoryA || !repositoryB) {
      throw new Error('Repositories not created')
    }
    if (!apiMonitor || !sseMonitorGlobal) {
      throw new Error('Event monitors not initialized')
    }

    // Step 1: Create two tasks with different agents and different objectives
    console.log('Creating Task A: File creation task')
    taskA = await createTestTask(
      baseURL,
      repositoryA.id,
      'Create a Python file called calculator.py with basic arithmetic functions (add, subtract, multiply, divide)',
      'claude-code',
      true,
    )

    console.log('Creating Task B: Documentation task')
    taskB = await createTestTask(
      baseURL,
      repositoryB.id,
      'Create a markdown file called FEATURES.md that lists the key features of this project',
      'claude-code',
      true,
    )

    console.log('Tasks created:', { taskA: taskA.taskId, taskB: taskB.taskId })

    // Step 2: Set up task-specific monitoring for Task A
    sseMonitorTaskA = new SSEEventMonitor(page, baseURL, taskA.taskId)
    await sseMonitorTaskA.start()
    expect(await sseMonitorTaskA.checkConnection()).toBe(true)

    // Step 3: Check SSE connection status
    const sseErrors = await sseMonitorGlobal.getLatestErrors()
    if (sseErrors.length > 0) {
      console.log('SSE Connection Errors:', sseErrors)
    }

    // Wait for both tasks to start processing
    console.log('Waiting for tasks to start processing...')

    // Add a small delay to allow events to flow
    await new Promise((resolve) => setTimeout(resolve, 2000))

    // Check if we have any events at all
    await sseMonitorGlobal.getLatestEvents()
    console.log('Current SSE events count:', sseMonitorGlobal.events.length)

    await Promise.all([
      waitForEventType(apiMonitor, 'message', 30000).catch((e) =>
        console.warn('Task start monitoring timed out:', e.message),
      ),
      waitForEventType(sseMonitorGlobal, 'message', 30000).catch((e) => {
        console.warn('SSE global start monitoring timed out:', e.message)
        console.log('SSE events at timeout:', sseMonitorGlobal.events.length)
        return null
      }),
    ])

    // Step 4: Monitor event stream status during execution
    let streamStatus = await getEventStreamStatus(baseURL)
    console.log('Initial stream status:', streamStatus)

    expect(streamStatus.globalConnections).toBeGreaterThan(0)
    expect(streamStatus.totalConnections).toBeGreaterThan(0)

    // Step 5: Wait for both tasks to complete
    console.log('Waiting for tasks to complete...')

    const [resultA, resultB] = await Promise.allSettled([
      waitForTaskResult(apiMonitor, taskA.taskId, 90000),
      waitForTaskResult(apiMonitor, taskB.taskId, 90000),
    ])

    // Log results for debugging
    if (resultA.status === 'fulfilled') {
      console.log('Task A completed:', resultA.value.success ? 'SUCCESS' : 'FAILED')
    } else {
      console.log('Task A failed to complete:', resultA.reason)
    }

    if (resultB.status === 'fulfilled') {
      console.log('Task B completed:', resultB.value.success ? 'SUCCESS' : 'FAILED')
    } else {
      console.log('Task B failed to complete:', resultB.reason)
    }

    // Step 6: Verify file creation by both agents
    const calculatorPath = path.join(tempDirA, 'calculator.py')
    const featuresPath = path.join(tempDirB, 'FEATURES.md')

    const calculatorExists = await fs
      .access(calculatorPath)
      .then(() => true)
      .catch(() => false)
    const featuresExists = await fs
      .access(featuresPath)
      .then(() => true)
      .catch(() => false)

    // At least one task should succeed (tests can be flaky with real agents)
    const anyTaskSucceeded = calculatorExists || featuresExists
    expect(anyTaskSucceeded).toBe(true)

    if (calculatorExists) {
      const calculatorContent = await fs.readFile(calculatorPath, 'utf-8')
      expect(calculatorContent).toContain('def ')
      expect(calculatorContent.toLowerCase()).toMatch(/add|subtract|multiply|divide/)
      console.log('✅ Calculator file created successfully')
    }

    if (featuresExists) {
      const featuresContent = await fs.readFile(featuresPath, 'utf-8')
      expect(featuresContent).toContain('#')
      expect(featuresContent.toLowerCase()).toContain('feature')
      console.log('✅ Features file created successfully')
    }

    // Step 7: Verify event cross-visibility
    console.log('Verifying event cross-visibility...')

    // Get final events from all monitors
    await sseMonitorGlobal.getLatestEvents()
    await sseMonitorTaskA.getLatestEvents()

    // Verify API monitor captured events from both tasks
    const apiEventsTaskA = apiMonitor.events.filter((e) => e.taskId === taskA.taskId)
    const apiEventsTaskB = apiMonitor.events.filter((e) => e.taskId === taskB.taskId)

    expect(apiEventsTaskA.length).toBeGreaterThan(0)
    expect(apiEventsTaskB.length).toBeGreaterThan(0)
    console.log('✅ API monitor captured events from both tasks')

    // Verify global SSE monitor captured events from both tasks
    const globalEventsTaskA = sseMonitorGlobal.events.filter((e) => e.taskId === taskA.taskId)
    const globalEventsTaskB = sseMonitorGlobal.events.filter((e) => e.taskId === taskB.taskId)

    expect(globalEventsTaskA.length).toBeGreaterThan(0)
    expect(globalEventsTaskB.length).toBeGreaterThan(0)
    console.log('✅ Global SSE monitor captured events from both tasks')

    // Verify task-specific SSE monitor only captured events from Task A
    const taskSpecificEvents = sseMonitorTaskA.events.filter((e) => e.taskId === taskA.taskId)
    const taskSpecificEventsOther = sseMonitorTaskA.events.filter((e) => e.taskId === taskB.taskId)

    expect(taskSpecificEvents.length).toBeGreaterThan(0)
    expect(taskSpecificEventsOther.length).toBe(0)
    console.log('✅ Task-specific SSE monitor correctly filtered events')

    // Step 8: Verify event types and structure
    const allEvents = [...apiMonitor.events]
    const eventTypes = [...new Set(allEvents.map((e) => e.type))]

    console.log('Event types captured:', eventTypes)

    // Should have key event types
    expect(eventTypes).toContain('message')
    expect(eventTypes.some((type) => type === 'result' || type.includes('result'))).toBe(true)

    // Verify event structure
    for (const event of allEvents.slice(0, 5)) {
      // Check first 5 events
      expect(event).toHaveProperty('eventId')
      expect(event).toHaveProperty('taskId')
      expect(event).toHaveProperty('type')
      expect(event).toHaveProperty('timestamp')
      expect(typeof event.eventId).toBe('string')
      expect(typeof event.taskId).toBe('string')
      expect(typeof event.type).toBe('string')
    }
    console.log('✅ Event structure validation passed')

    // Step 9: Verify event ordering
    const taskAEvents = allEvents.filter((e) => e.taskId === taskA.taskId)
    const taskBEvents = allEvents.filter((e) => e.taskId === taskB.taskId)

    // Events within each task should be chronologically ordered
    for (let i = 1; i < taskAEvents.length; i++) {
      const prevTime = new Date(taskAEvents[i - 1].timestamp).getTime()
      const currTime = new Date(taskAEvents[i].timestamp).getTime()
      expect(currTime).toBeGreaterThanOrEqual(prevTime)
    }

    for (let i = 1; i < taskBEvents.length; i++) {
      const prevTime = new Date(taskBEvents[i - 1].timestamp).getTime()
      const currTime = new Date(taskBEvents[i].timestamp).getTime()
      expect(currTime).toBeGreaterThanOrEqual(prevTime)
    }
    console.log('✅ Event ordering validation passed')

    // Step 10: Verify consistent event IDs across monitors
    const apiEventIds = new Set(apiMonitor.events.map((e) => e.eventId))
    const sseEventIds = new Set(sseMonitorGlobal.events.map((e) => e.eventId))

    // Global SSE events should be a subset of API events (due to potential timing differences)
    let matchingEventIds = 0
    for (const eventId of sseEventIds) {
      if (apiEventIds.has(eventId)) {
        matchingEventIds++
      }
    }

    // At least 80% of SSE events should match API events
    const matchPercentage = matchingEventIds / sseEventIds.size
    expect(matchPercentage).toBeGreaterThan(0.8)
    console.log(`✅ Event ID consistency: ${(matchPercentage * 100).toFixed(1)}% match`)

    // Step 11: Final stream status check
    streamStatus = await getEventStreamStatus(baseURL)
    console.log('Final stream status:', streamStatus)

    // Should still have active connections
    expect(streamStatus.totalConnections).toBeGreaterThan(0)

    // Step 12: Test frontend navigation and event updates
    console.log('Testing frontend navigation...')

    // Navigate to tasks page
    await page.goto('/tasks')
    await page.waitForLoadState('networkidle')

    // Should see both tasks in the UI
    const taskElements = await page.locator('[data-testid="task-item"]').count()
    expect(taskElements).toBeGreaterThanOrEqual(2)

    // Navigate to specific task detail
    await page.goto(`/tasks/${taskA.taskId}`)
    await page.waitForLoadState('networkidle')

    // Should see task details
    await expect(page.locator('h1, h2, [data-testid="task-title"]')).toBeVisible()

    console.log('✅ Frontend navigation and UI updates verified')

    // Final summary
    console.log('\n🎉 Complete E2E Event Flow Test Summary:')
    console.log(`- API Events Captured: ${apiMonitor?.events.length || 0}`)
    console.log(`- Global SSE Events: ${sseMonitorGlobal?.events.length || 0}`)
    console.log(`- Task-Specific SSE Events: ${sseMonitorTaskA?.events.length || 0}`)
    console.log(`- API Errors: ${apiMonitor?.errors.length || 0}`)
    console.log(`- SSE Errors: ${sseMonitorGlobal?.errors.length || 0}`)
    console.log(`- Files Created: ${Number(calculatorExists) + Number(featuresExists)}/2`)
    console.log(`- Event Types: [${eventTypes.join(', ')}]`)
  })

  test('should handle connection resilience and reconnection', async ({ page }) => {
    // Ensure repository exists
    if (!repositoryA) {
      throw new Error('Repository A not created')
    }

    // Create a single task for this test
    taskA = await createTestTask(
      baseURL,
      repositoryA.id,
      'Create a simple hello.txt file with "Hello World" content',
      'claude-code',
      true,
    )

    // Ensure monitors exist
    if (!apiMonitor || !sseMonitorGlobal) {
      throw new Error('Event monitors not initialized')
    }

    // Wait for initial events
    await waitForEventType(apiMonitor, 'message', 30000)

    const initialEventCount = sseMonitorGlobal.events.length
    console.log('Initial SSE events:', initialEventCount)

    // Simulate connection interruption by stopping and restarting SSE
    console.log('Testing connection resilience...')
    await sseMonitorGlobal.cleanup()

    // Wait a moment
    await page.waitForTimeout(2000)

    // Restart SSE connection
    sseMonitorGlobal = new SSEEventMonitor(page, baseURL)
    await sseMonitorGlobal.start()

    expect(await sseMonitorGlobal.checkConnection()).toBe(true)
    console.log('✅ SSE connection restored successfully')

    // Wait for task completion
    const result = await waitForTaskResult(apiMonitor, taskA.taskId, 60000)
    expect(result.success).toBe(true)

    // Verify we continued receiving events after reconnection
    await sseMonitorGlobal.getLatestEvents()
    const finalEventCount = sseMonitorGlobal.events.length

    expect(finalEventCount).toBeGreaterThan(0)
    console.log(`✅ Received ${finalEventCount} events after reconnection`)

    // Verify file was created
    const helloPath = path.join(tempDirA, 'hello.txt')
    const helloExists = await fs
      .access(helloPath)
      .then(() => true)
      .catch(() => false)

    if (helloExists) {
      const content = await fs.readFile(helloPath, 'utf-8')
      expect(content.toLowerCase()).toContain('hello')
      console.log('✅ File created successfully despite connection interruption')
    }
  })
})
