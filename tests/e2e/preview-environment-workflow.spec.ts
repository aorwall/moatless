import { test, expect } from '@playwright/test'
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { setupTempGitRepo, createTestRepository, deleteTestTask, deleteTestRepository } from './helpers/test-server'

/**
 * Preview Environment E2E Test
 * 
 * This test validates the preview environment functionality:
 * 1. Creates a repository via API (backend setup)
 * 2. Uses the UI to create a task that clones a UI repository
 * 3. Monitors the task until the environment is created
 * 4. Navigates to the preview URL
 * 5. Verifies the application is running correctly
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
    repository = await createTestRepository(
      baseURL,
      'Preview E2E Test Repository',
      tempDir
    )
    
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

  test('should clone hello-world-ui and verify preview environment', async ({ page }) => {
    // Step 1: Navigate to the homepage
    await page.goto('/', { waitUntil: 'domcontentloaded' })
    
    // Step 2: Wait for the chat input to be visible
    const textareaSelector = 'textarea'
    await page.waitForSelector(textareaSelector, {
      state: 'visible',
      timeout: 30000
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
    const taskMessage = 'Clone https://github.com/aorwall/hello-world-ui and run it with "pnpm run dev"'
    const chatInput = page.locator('textarea').first()
    await chatInput.fill(taskMessage)
    
    console.log('✅ Task message entered')
    
    // Step 5: Click the submit button to create the task
    const submitButton = page.locator('button').filter({ hasText: 'Start Task' }).first()
    await expect(submitButton).toBeEnabled({ timeout: 5000 })
    
    // Listen for the navigation to capture the task ID
    const navigationPromise = page.waitForNavigation({
      url: /\/tasks\/[a-zA-Z0-9-]+/,
      waitUntil: 'domcontentloaded'
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
    
    // Step 7: Monitor task execution and wait for environment creation
    const maxWaitTime = 120000 // 120 seconds - cloning and pnpm install can take time
    const startTime = Date.now()
    let environmentCreated = false
    let previewUrl: string | undefined
    
    console.log('⏳ Waiting for environment to be created...')
    
    while (!environmentCreated && (Date.now() - startTime) < maxWaitTime) {
      // Check task status via API
      const taskResponse = await page.request.get(`${baseURL}/api/v1/tasks/${createdTaskId}`)
      if (taskResponse.ok()) {
        const taskData = await taskResponse.json()
        
        // Check if task has an associated environment
        if (taskData.environment_id) {
          console.log('✅ Environment created:', taskData.environment_id)
          
          // Get environment details
          const envResponse = await page.request.get(`${baseURL}/api/v1/environments/${taskData.environment_id}`)
          if (envResponse.ok()) {
            const envData = await envResponse.json()
            if (envData.preview_url) {
              previewUrl = envData.preview_url
              environmentCreated = true
              console.log('✅ Preview URL available:', previewUrl)
              break
            }
          }
        }
        
        // Also check if the agent has completed cloning and running the app
        if (taskData.iterations && taskData.iterations.length > 0) {
          const lastIteration = taskData.iterations[taskData.iterations.length - 1]
          if (lastIteration.result && 
              (lastIteration.result.includes('hello-world-ui') || 
               lastIteration.result.includes('pnpm run dev') ||
               lastIteration.result.includes('development server'))) {
            console.log('✅ Agent has processed the clone request')
          }
        }
      }
      
      await page.waitForTimeout(3000) // Wait 3 seconds before checking again
    }
    
    expect(environmentCreated).toBe(true)
    expect(previewUrl).toBeDefined()
    
    // Step 8: Navigate to the preview tab
    await page.locator('button:has-text("Preview")').click()
    
    // Wait for preview panel to be visible
    const previewPanel = page.locator('[value="preview"]')
    await expect(previewPanel).toBeVisible()
    
    console.log('✅ Preview tab opened')
    
    // Step 9: Wait for the development server to be ready
    // The agent needs time to install dependencies and start the server
    console.log('⏳ Waiting for development server to start...')
    await page.waitForTimeout(20000) // Give pnpm install and dev server time to start
    
    // Step 10: Navigate to the preview URL in the iframe or new tab
    // The preview might be in an iframe or might need to open in a new tab
    const previewFrame = page.frameLocator('iframe[title="Preview"]')
    
    // Try to interact with the preview content
    try {
      // Check if we can see the expected content in the iframe
      await expect(previewFrame.locator('text="Your App"')).toBeVisible({ timeout: 30000 })
      await expect(previewFrame.locator('text="Start building something amazing"')).toBeVisible({ timeout: 5000 })
      
      console.log('✅ Preview environment is running correctly!')
    } catch (e) {
      // If iframe doesn't work, try opening the preview URL directly
      console.log('⚠️ Could not verify content in iframe, trying direct navigation...')
      
      if (previewUrl) {
        // Open preview in a new page
        const previewPage = await page.context().newPage()
        await previewPage.goto(previewUrl, { waitUntil: 'domcontentloaded' })
        
        // Wait for the app to load
        await previewPage.waitForSelector('text="Your App"', { timeout: 30000 })
        await expect(previewPage.locator('text="Your App"')).toBeVisible()
        await expect(previewPage.locator('text="Start building something amazing"')).toBeVisible()
        
        console.log('✅ Preview environment verified via direct navigation!')
        
        // Take a screenshot of the preview
        await previewPage.screenshot({ 
          path: `test-results/preview-environment-${createdTaskId}.png`,
          fullPage: true 
        })
        
        await previewPage.close()
      } else {
        throw new Error('Preview URL not available')
      }
    }
    
    // Step 11: Verify environment health status
    const envId = await (async () => {
      const taskResponse = await page.request.get(`${baseURL}/api/v1/tasks/${createdTaskId}`)
      const taskData = await taskResponse.json()
      return taskData.environment_id
    })()
    
    if (envId) {
      const healthResponse = await page.request.get(`${baseURL}/api/v1/environments/${envId}/health`)
      if (healthResponse.ok()) {
        const healthData = await healthResponse.json()
        console.log('✅ Environment health:', healthData)
        
        // Verify the server is healthy
        expect(healthData.status).toBe('healthy')
        expect(healthData.serverStatus).toBe('running')
      }
    }
    
    // Step 12: Check the UI for environment status indicators
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
      fullPage: true 
    })
    
    console.log('✅ Preview environment E2E test completed successfully!')
  }, 150000) // 150 second timeout for the entire test (cloning and installing can be slow)
  
  test('should handle preview environment errors gracefully', async ({ page }) => {
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
    
    // Verify no environment was created for invalid repository
    expect(taskData.environment_id).toBeUndefined()
    
    console.log('✅ Preview environment error handling verified')
  })
})