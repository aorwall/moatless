import { test, expect } from '@playwright/test'
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { setupTempGitRepo, createTestRepository, deleteTestTask, deleteTestRepository } from './helpers/test-server'

/**
 * Simple Preview Environment E2E Test
 * 
 * This test validates basic task creation for cloning repositories:
 * 1. Creates a repository via API
 * 2. Uses the UI to create a task that clones a repository
 * 3. Verifies the task is created and agent starts working
 * 4. Checks that the agent makes progress
 */

test.describe('Simple Preview Environment Test', () => {
  let tempDir: string
  let repository: { id: string; name: string }
  let createdTaskId: string | undefined
  const baseURL = `http://127.0.0.1:${globalThis.__E2E_SERVER_PORT__ || 3002}`

  test.beforeEach(async () => {
    // Create a temporary directory for our test git repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-preview-simple-test-'))
    
    // Initialize a git repository in the temp directory
    await setupTempGitRepo(tempDir)
    
    // Create a test repository via API
    repository = await createTestRepository(
      baseURL,
      'Simple Preview Test Repository',
      tempDir
    )
    
    console.log('✅ Test repository created:', repository.id)
  })

  test.afterEach(async () => {
    // Cleanup task if it exists
    if (createdTaskId) {
      // Stop the task first
      await fetch(`${baseURL}/api/v1/tasks/${createdTaskId}/stop`, { method: 'POST' })
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

  test('should create task and verify agent starts working', async ({ page }) => {
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
    
    // Step 4: Type the task message
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
    
    // Step 7: Monitor for 30 seconds to see if agent makes progress
    const monitorTime = 30000 // 30 seconds
    const startTime = Date.now()
    let hasIterations = false
    let iterationCount = 0
    
    console.log('⏳ Monitoring agent progress for 30 seconds...')
    
    while ((Date.now() - startTime) < monitorTime) {
      // Check task status via API
      const taskResponse = await page.request.get(`${baseURL}/api/v1/tasks/${createdTaskId}`)
      if (taskResponse.ok()) {
        const taskData = await taskResponse.json()
        
        // Check if the agent has any iterations
        if (taskData.iterations && taskData.iterations.length > 0) {
          hasIterations = true
          if (taskData.iterations.length > iterationCount) {
            iterationCount = taskData.iterations.length
            const lastIteration = taskData.iterations[taskData.iterations.length - 1]
            console.log(`Iteration ${iterationCount}: ${lastIteration.status}`)
            
            // Log the first few characters of the result
            if (lastIteration.result) {
              const preview = lastIteration.result.substring(0, 100).replace(/\n/g, ' ')
              console.log(`  Result preview: ${preview}...`)
            }
          }
        }
        
        // Check task status
        if (taskData.status === 'failed') {
          console.log('❌ Task failed:', taskData.error || 'Unknown error')
          // Don't throw error, we just want to verify the agent started
        }
      }
      
      await page.waitForTimeout(5000) // Wait 5 seconds before checking again
    }
    
    // Verify the agent at least started working
    expect(hasIterations).toBe(true)
    expect(iterationCount).toBeGreaterThan(0)
    
    console.log(`✅ Agent created ${iterationCount} iterations in 30 seconds`)
    
    // Step 8: Check that UI shows task activity
    // Look for any indication that the task is processing
    const taskContent = page.locator('main')
    
    // Check for iteration indicators in the UI
    const hasContent = await taskContent.locator('text=/Tool Result|Assistant|Bash/').count() > 0
    expect(hasContent).toBe(true)
    
    console.log('✅ Task UI shows agent activity')
    
    // Take a screenshot of the task
    await page.screenshot({ 
      path: `test-results/simple-preview-task-${createdTaskId}.png`,
      fullPage: true 
    })
    
    console.log('✅ Simple preview environment test completed successfully!')
  }, 60000) // 60 second timeout for the entire test
})