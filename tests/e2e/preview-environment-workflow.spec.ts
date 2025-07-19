import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { expect, test } from '@playwright/test'
import {
  createTestRepository,
  deleteTestRepository,
  deleteTestTask,
  setupTempGitRepo,
} from './helpers/test-server'

/**
 * Preview Environment E2E Test
 *
 * This test validates the development server functionality:
 * 1. Creates a repository via API (backend setup)
 * 2. Uses the UI to create a task that clones a UI repository
 * 3. Monitors the task until the development server starts
 * 4. Verifies the server is running by checking server logs
 * 5. Validates the agent successfully started the dev server
 */

test.describe('Preview Environment Workflow', () => {
  let tempDir: string
  let repository: { id: string; name: string }
  let createdTaskId: string | undefined
  const baseURL = `http://127.0.0.1:${globalThis.__E2E_SERVER_PORT__ || 3002}`

  test.beforeEach(async () => {
    // Create a temporary directory for our test git repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-preview-e2e-test-'))

    // Initialize a git repository in the temp directory
    await setupTempGitRepo(tempDir)

    // Create a test repository via API
    repository = await createTestRepository(baseURL, 'Preview E2E Test Repository', tempDir)

    console.log('✅ Test repository created:', repository.id)
  })

  test.afterEach(async () => {
    // Cleanup task if it exists
    if (createdTaskId) {
      await deleteTestTask(baseURL, createdTaskId)
    }

    // Cleanup repository
    if (repository?.id) {
      await deleteTestRepository(baseURL, repository.id)
    }

    // Clean up temp directory
    if (tempDir) {
      try {
        await fs.rm(tempDir, { recursive: true, force: true })
      } catch (_e) {
        // Directory might already be deleted
      }
    }
  })

  test('should clone hello-world-ui and run development server', async ({ page }) => {
    // Step 1: Navigate to the homepage
    await page.goto('/', { waitUntil: 'domcontentloaded' })

    // Step 2: Wait for the chat input to be visible
    const textareaSelector = 'textarea'
    await page.waitForSelector(textareaSelector, {
      state: 'visible',
      timeout: 30000,
    })

    console.log('✅ Homepage loaded, chat input visible')

    // Step 3: Select the repository from dropdown
    const selectSelector = 'select'
    await page.waitForSelector(selectSelector, { state: 'visible', timeout: 10000 })

    const selectElement = page.locator(selectSelector).first()

    // Wait a moment for options to load
    await page.waitForTimeout(1000)

    // Select our repository
    await selectElement.selectOption({ label: repository.name })

    console.log('✅ Repository selected:', repository.name)

    // Step 4: Type the task message to clone hello-world-ui
    const taskMessage =
      'Clone https://github.com/aorwall/hello-world-ui and run it with "bun run dev"'
    const chatInput = page.locator('textarea').first()
    await chatInput.fill(taskMessage)

    console.log('✅ Task message entered')

    // Step 5: Click the submit button to create the task
    const submitButton = page.locator('button').filter({ hasText: 'Start Task' }).first()
    await expect(submitButton).toBeEnabled({ timeout: 5000 })

    // Listen for the navigation to capture the task ID
    const navigationPromise = page.waitForNavigation({
      url: /\/tasks\/[a-zA-Z0-9-]+/,
      waitUntil: 'domcontentloaded',
    })

    await submitButton.click()

    // Wait for navigation to task detail page
    await navigationPromise

    // Extract task ID from URL
    const url = page.url()
    const taskIdMatch = url.match(/\/tasks\/([a-zA-Z0-9-]+)/)
    if (taskIdMatch) {
      createdTaskId = taskIdMatch[1]
      console.log('✅ Task created with ID:', createdTaskId)
    } else {
      throw new Error('Failed to extract task ID from URL')
    }

    // Step 6: Wait for task page to load
    await page.waitForLoadState('domcontentloaded')

    // Wait a bit for React to render
    await page.waitForTimeout(2000)

    console.log('✅ Task detail page loaded')

    // Step 7: Monitor task execution and wait for development server to start
    const maxWaitTime = 120000 // 120 seconds - cloning and bun install can take time
    const startTime = Date.now()
    let serverStarted = false
    let iterationCount = 0

    console.log('⏳ Waiting for development server to start...')

    while (!serverStarted && Date.now() - startTime < maxWaitTime) {
      // Check task status via API
      const taskResponse = await page.request.get(`${baseURL}/api/v1/tasks/${createdTaskId}`)
      if (taskResponse.ok()) {
        const taskData = await taskResponse.json()

        // Check if the agent has completed cloning and started the dev server
        if (taskData.iterations && taskData.iterations.length > iterationCount) {
          iterationCount = taskData.iterations.length
          const lastIteration = taskData.iterations[taskData.iterations.length - 1]

          console.log(`Iteration ${iterationCount}: ${lastIteration.status}`)

          if (lastIteration.result) {
            // Log what the agent is doing
            if (lastIteration.result.includes('git clone')) {
              console.log('✅ Repository cloned')
            } else if (lastIteration.result.includes('bun install')) {
              console.log('✅ Dependencies being installed')
            } else if (
              lastIteration.result.includes('bun run dev') ||
              lastIteration.result.includes('VITE') ||
              lastIteration.result.includes('Local:') ||
              lastIteration.result.includes('localhost:')
            ) {
              console.log('✅ Development server started!')
              serverStarted = true
              break
            }
          }
        }

        // Check task status
        if (taskData.status === 'failed' || taskData.status === 'cancelled') {
          throw new Error(`Task ${taskData.status}: ${taskData.error || 'Unknown error'}`)
        }
      }

      await page.waitForTimeout(3000) // Wait 3 seconds before checking again
    }

    expect(serverStarted).toBe(true)

    // Step 8: Check the Server tab to see if dev server is running
    await page.locator('button:has-text("Server")').click()

    // Wait for server panel to be visible
    const serverPanel = page.locator('[value="server"]')
    await expect(serverPanel).toBeVisible()

    console.log('✅ Server tab opened')

    // Wait a bit for server logs to appear
    await page.waitForTimeout(2000)

    // Check if we can see server logs indicating the dev server is running
    const serverLogs = page.locator('[value="server"]')

    // Look for indicators that the Vite dev server is running
    const viteIndicators = ['VITE', 'Local:', 'localhost:', 'ready in', 'dev server running']

    let foundServerRunning = false
    for (const indicator of viteIndicators) {
      try {
        const element = serverLogs.locator(`text=${indicator}`)
        if (await element.isVisible({ timeout: 5000 })) {
          console.log(`✅ Found server indicator: ${indicator}`)
          foundServerRunning = true
          break
        }
      } catch (e) {
        // Continue checking other indicators
      }
    }

    expect(foundServerRunning).toBe(true)

    // Step 9: Check the UI for task completion
    // Look for environment status in the task page
    await page.locator('button:has-text("Files")').click()

    // Verify we can see the cloned repository files
    const filesPanel = page.locator('[value="files"]')
    await expect(filesPanel).toBeVisible()

    // Look for hello-world-ui files
    const packageJsonFile = page.locator('text="package.json"')
    await expect(packageJsonFile).toBeVisible({ timeout: 10000 })

    console.log('✅ Cloned repository files visible in file tree')

    // Take a final screenshot of the completed task
    await page.screenshot({
      path: `test-results/preview-environment-task-${createdTaskId}.png`,
      fullPage: true,
    })

    console.log('✅ Development server E2E test completed successfully!')
  }, 150000) // 150 second timeout for the entire test (cloning and installing can be slow)

  test('should handle repository cloning errors gracefully', async ({ page }) => {
    // Navigate to homepage
    await page.goto('/', { waitUntil: 'domcontentloaded' })

    // Wait for initial load
    await page.waitForTimeout(3000)

    // Select repository and create a task with invalid repository URL
    const selectElement = page.locator('select').first()
    await selectElement.selectOption({ label: repository.name })

    const chatInput = page.locator('textarea').first()
    await chatInput.fill('Clone https://github.com/invalid/non-existent-repo and run it')

    const submitButton = page.locator('button:has-text("Start Task")').first()
    await submitButton.click()

    // Wait for navigation
    await page.waitForURL(/\/tasks\/[a-zA-Z0-9-]+/)

    // Extract task ID
    const url = page.url()
    const taskIdMatch = url.match(/\/tasks\/([a-zA-Z0-9-]+)/)
    if (taskIdMatch) {
      createdTaskId = taskIdMatch[1]
    }

    // Wait for task to process
    await page.waitForTimeout(10000)

    // Check that the task handles the error appropriately
    // The agent should report that it couldn't clone the repository
    const taskResponse = await page.request.get(`${baseURL}/api/v1/tasks/${createdTaskId}`)
    const taskData = await taskResponse.json()

    // Verify the task shows an error or is still running without success
    if (taskData.iterations && taskData.iterations.length > 0) {
      const lastIteration = taskData.iterations[taskData.iterations.length - 1]
      // Should have an error message about the repository not being found
      expect(lastIteration.result).toMatch(/error|failed|not found|does not exist/i)
    }

    console.log('✅ Repository cloning error handling verified')
  })
})
