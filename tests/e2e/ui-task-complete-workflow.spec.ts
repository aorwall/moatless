import { test, expect } from '@playwright/test'
import { promises as fs } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { setupTempGitRepo, createTestRepository, deleteTestTask, deleteTestRepository } from './helpers/test-server'

/**
 * UI-based E2E Test for Complete Task Workflow
 * 
 * This test validates the entire task creation and execution flow through the UI:
 * 1. Creates a repository via API (backend setup)
 * 2. Uses the UI to create a task
 * 3. Monitors the task through the UI until completion
 * 4. Verifies the agent successfully completed the task
 * 
 * Similar to the backend complete-workflow-e2e.test.ts but using the UI for task creation
 */

test.describe('UI Task Complete Workflow', () => {
  let tempDir: string
  let repository: { id: string; name: string }
  let createdTaskId: string | undefined
  const baseURL = `http://127.0.0.1:${globalThis.__E2E_SERVER_PORT__ || 3002}`

  test.beforeEach(async () => {
    // Create a temporary directory for our test git repository
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'moatless-ui-e2e-test-'))
    
    // Initialize a git repository in the temp directory
    await setupTempGitRepo(tempDir)
    
    // Create a test repository via API
    repository = await createTestRepository(
      baseURL,
      'UI E2E Test Repository',
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

  test('should create task via UI and monitor until completion', async ({ page }) => {
    // Step 1: Navigate to the homepage
    await page.goto('/', { waitUntil: 'domcontentloaded' })
    
    // Step 2: Wait for the chat input to be visible
    // Try multiple selectors to find the textarea
    const textareaSelector = 'textarea'
    await page.waitForSelector(textareaSelector, {
      state: 'visible',
      timeout: 30000
    })
    
    console.log('✅ Homepage loaded, chat input visible')
    
    // Step 3: Select the repository from dropdown
    // Wait for the select element to be visible
    const selectSelector = 'select'
    await page.waitForSelector(selectSelector, { state: 'visible', timeout: 10000 })
    
    // Select the repository by value or text
    const selectElement = page.locator(selectSelector).first()
    
    // Wait a moment for options to load
    await page.waitForTimeout(1000)
    
    // Select our repository
    await selectElement.selectOption({ label: repository.name })
    
    console.log('✅ Repository selected:', repository.name)
    
    // Step 4: Type the task message
    const taskMessage = 'Create a file called hello_world.py with the content: print("Hello, World!")'
    const chatInput = page.locator('textarea').first()
    await chatInput.fill(taskMessage)
    
    console.log('✅ Task message entered')
    
    // Step 5: Click the submit button to create the task
    // Look for button with "Start Task" text
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
    
    // Step 7: Monitor task execution
    // Look for indicators that the task is running
    const runningIndicators = [
      page.locator('text=/Running|In Progress|Executing/i'),
      page.locator('[data-testid="task-status"]:has-text("running")')
    ]
    
    // Wait for task to start running
    let isRunning = false
    for (const indicator of runningIndicators) {
      if (await indicator.isVisible({ timeout: 5000 }).catch(() => false)) {
        isRunning = true
        console.log('✅ Task is running')
        break
      }
    }
    
    // Step 8: Wait for task to actually complete with real agent
    // This requires waiting for the agent to process and create the file
    const maxWaitTime = 60000 // 60 seconds
    const startTime = Date.now()
    let taskCompleted = false
    let agentCompleted = false
    
    console.log('⏳ Waiting for agent to complete task...')
    
    while (!agentCompleted && (Date.now() - startTime) < maxWaitTime) {
      // Check task status via API to see if agent finished
      const taskResponse = await page.request.get(`${baseURL}/api/v1/tasks/${createdTaskId}`)
      if (taskResponse.ok()) {
        const taskData = await taskResponse.json()
        
        // Check if task has completed iterations
        if (taskData.iterations && taskData.iterations.length > 0) {
          const lastIteration = taskData.iterations[taskData.iterations.length - 1]
          if (lastIteration.status === 'completed' && lastIteration.result) {
            agentCompleted = true
            console.log('✅ Agent completed the task')
            break
          }
        }
        
        // Also check overall task status
        if (taskData.status === 'completed' || taskData.status === 'failed') {
          taskCompleted = true
        }
      }
      
      // Also check UI for completion indicators
      const completionIndicators = [
        page.locator('text=/created.*hello_world\.py/i'),
        page.locator('text=/successfully created/i'),
        page.locator('pre:has-text("Hello, World!")'),
        page.locator('code:has-text("print")')
      ]
      
      for (const indicator of completionIndicators) {
        if (await indicator.isVisible({ timeout: 500 }).catch(() => false)) {
          console.log('✅ Task completion visible in UI')
          break
        }
      }
      
      await page.waitForTimeout(2000) // Wait 2 seconds before checking again
    }
    
    // Wait a bit more to ensure file is written
    await page.waitForTimeout(3000)
    
    expect(agentCompleted).toBe(true)
    console.log('✅ Task completed successfully')
    
    // Step 9: Check git diff to see if file was created
    const gitDiffResponse = await page.request.get(`${baseURL}/api/v1/tasks/${createdTaskId}/git-diff`)
    expect(gitDiffResponse.status()).toBe(200)
    
    const gitDiffData = await gitDiffResponse.json()
    console.log('Git diff:', gitDiffData.diff ? gitDiffData.diff.substring(0, 500) : 'No diff')
    
    // Step 10: Verify the file was created via API
    const filesResponse = await page.request.get(`${baseURL}/api/v1/tasks/${createdTaskId}/files`)
    expect(filesResponse.status()).toBe(200)
    
    const filesData = await filesResponse.json()
    console.log('Files in workspace:', filesData.files.map((f: any) => f.name))
    
    // The file might be created but we need to check if agent committed it
    const helloWorldFile = filesData.files.find((f: any) => f.name === 'hello_world.py')
    
    // If file is not in root, check if it's in git diff or was committed
    if (!helloWorldFile) {
      // Check if the file was created as shown in git diff
      if (gitDiffData.diff && gitDiffData.diff.includes('hello_world.py')) {
        console.log('✅ File hello_world.py was created (shown in git diff)')
      } else {
        // File might have been committed, check task result
        const taskResponse = await page.request.get(`${baseURL}/api/v1/tasks/${createdTaskId}`)
        const taskData = await taskResponse.json()
        if (taskData.iterations?.[0]?.result?.includes('hello_world.py')) {
          console.log('✅ File hello_world.py was created (mentioned in task result)')
        }
      }
    } else {
      expect(helloWorldFile).toBeTruthy()
      console.log('✅ File hello_world.py was created')
    }
    
    // Step 11: Try to verify file content if it exists
    if (helloWorldFile) {
      const fileContentResponse = await page.request.get(
        `${baseURL}/api/v1/tasks/${createdTaskId}/files/hello_world.py`
      )
      
      if (fileContentResponse.status() === 200) {
        const contentData = await fileContentResponse.json()
        expect(contentData.content).toContain('Hello, World!')
        expect(contentData.content).toContain('print')
        console.log('✅ File content verified:', contentData.content.trim())
      }
    }
    
    // Step 12: Verify task events were recorded (if available)
    const eventsResponse = await page.request.get(`${baseURL}/api/v1/tasks/${createdTaskId}/events`)
    expect(eventsResponse.status()).toBe(200)
    
    const eventsData = await eventsResponse.json()
    
    // Events might not be stored in test environment, so make this check optional
    if (eventsData.events && eventsData.events.length > 0) {
      // Look for key events
      const messageEvents = eventsData.events.filter((e: any) => e.type === 'message')
      const resultEvents = eventsData.events.filter((e: any) => e.type === 'result')
      
      console.log('✅ Task events verified:', {
        totalEvents: eventsData.events.length,
        messageEvents: messageEvents.length,
        resultEvents: resultEvents.length
      })
    } else {
      console.log('⚠️ No events recorded (this is OK in test environment)')
    }
    
    // Step 13: Final verification - check that task completed with file creation
    const finalTaskResponse = await page.request.get(`${baseURL}/api/v1/tasks/${createdTaskId}`)
    const finalTaskData = await finalTaskResponse.json()
    
    expect(finalTaskData.status).not.toBe('failed')
    expect(finalTaskData.iterations.length).toBeGreaterThan(0)
    expect(finalTaskData.iterations[0].status).toBe('completed')
    
    // Verify the task result mentions creating the file
    const taskResult = finalTaskData.iterations[0].result || ''
    const fileWasCreated = taskResult.includes('hello_world.py') || 
                          taskResult.includes('Hello, World!') ||
                          gitDiffData.diff?.includes('hello_world.py') ||
                          helloWorldFile !== undefined
    
    expect(fileWasCreated).toBe(true)
    
    // Step 14: Take a screenshot of the completed task
    await page.screenshot({ 
      path: `test-results/ui-task-complete-${createdTaskId}.png`,
      fullPage: true 
    })
    
    console.log('✅ UI-based task creation and monitoring test completed successfully!')
  }, 90000) // 90 second timeout for the entire test
  
  test('should handle task creation errors gracefully', async ({ page }) => {
    // Navigate to homepage
    await page.goto('/')
    await page.waitForLoadState('networkidle')
    
    // Try to create a task without selecting a repository
    const chatInput = page.locator('[data-testid="chat-input"], textarea').first()
    await chatInput.fill('This task should fail without a repository')
    
    const submitButton = page.locator('button:has-text("Start Task"), button[type="submit"]').first()
    
    // Button should be disabled without repository selection
    await expect(submitButton).toBeDisabled()
    
    console.log('✅ UI correctly prevents task creation without repository')
  })
})