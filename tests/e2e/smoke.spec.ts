import { expect, test } from '@playwright/test'

/**
 * Essential smoke tests to validate the application is working correctly
 */

test.describe('Essential Application Tests', () => {
  test('should load the React application', async ({ page }) => {
    // Navigate to the application
    await page.goto('/')

    // Wait for the page to load
    await page.waitForLoadState('domcontentloaded')

    // Check that the React app root element exists and has been hydrated
    const appRoot = page.locator('#app')
    await expect(appRoot).toBeVisible()

    // Wait for React to render something inside the app root
    await expect(appRoot).not.toBeEmpty()

    // Check for essential UI elements that indicate the app loaded correctly
    // Look for the main layout structure
    const mainContent = page.locator('main')
    await expect(mainContent).toBeVisible({ timeout: 10000 })

    console.log('✅ React application loaded successfully')
  })

  test('should load the application with proper title', async ({ page }) => {
    await page.goto('/')

    // Check the page title - update this based on your actual app title
    await expect(page).toHaveTitle(/Moatless|Vibe/i, { timeout: 10000 })

    console.log('✅ Application title verified')
  })

  test('should access repositories API endpoint', async ({ page, baseURL }) => {
    // Test that the repositories endpoint returns the expected structure
    const response = await page.request.get(`${baseURL}/api/v1/repositories`)
    expect(response.status()).toBe(200)

    const data = await response.json()
    expect(data).toHaveProperty('repositories')
    expect(Array.isArray(data.repositories)).toBe(true)

    // When using a temporary data directory, the list should be empty
    console.log(`✅ Repositories API accessible - found ${data.repositories.length} repositories`)
  })

  test('should access settings API endpoint', async ({ page, baseURL }) => {
    // Test that the settings endpoint works
    const response = await page.request.get(`${baseURL}/api/v1/settings/status`)
    expect(response.status()).toBe(200)

    const status = await response.json()
    expect(status).toHaveProperty('docker')
    expect(status).toHaveProperty('claude')
    expect(status.docker).toHaveProperty('isRunning')
    expect(status.claude).toHaveProperty('isValidBin')

    console.log('✅ Settings API accessible')
    console.log(`  - Docker running: ${status.docker.isRunning}`)
    console.log(`  - Claude valid: ${status.claude.isValidBin}`)
  })

  test('should verify SSE event stream endpoint', async ({ page, baseURL }) => {
    // Navigate to the app first to establish context
    await page.goto('/')

    // Test SSE connection status endpoint
    const statusResponse = await page.request.get(`${baseURL}/api/v1/events/stream/status`)
    expect(statusResponse.status()).toBe(200)

    const status = await statusResponse.json()
    expect(status).toHaveProperty('globalConnections')
    expect(status).toHaveProperty('taskConnections')
    expect(status).toHaveProperty('totalConnections')

    // Test that SSE endpoint is accessible
    const sseTest = await page.evaluate(async (url) => {
      const controller = new AbortController()

      try {
        const response = await fetch(`${url}/api/v1/events/stream`, {
          headers: {
            Accept: 'text/event-stream',
          },
          signal: controller.signal,
        })

        // Abort immediately after checking the response
        setTimeout(() => controller.abort(), 10)

        return {
          status: response.status,
          contentType: response.headers.get('content-type'),
          ok: response.ok,
        }
      } catch (error: any) {
        // AbortError is expected
        if (error.name === 'AbortError') {
          return { status: 200, ok: true, aborted: true }
        }
        throw error
      }
    }, baseURL)

    expect(sseTest.ok).toBe(true)
    if (!sseTest.aborted) {
      expect(sseTest.status).toBe(200)
      expect(sseTest.contentType).toContain('text/event-stream')
    }

    console.log('✅ SSE event stream endpoint is accessible')
  })

  test('should handle temporary data directory', async ({ page, baseURL }) => {
    // Verify that the application is using the temporary data directory set by tests
    const settingsResponse = await page.request.get(`${baseURL}/api/v1/settings/general`)
    expect(settingsResponse.status()).toBe(200)

    const settingsData = await settingsResponse.json()
    
    // The test environment should be using E2E_TEMP_DIR
    if (process.env.E2E_TEMP_DIR) {
      console.log(`✅ Using temporary data directory: ${process.env.E2E_TEMP_DIR}`)
    }

    // Verify empty repositories list (since we're using a temp directory)
    const repoResponse = await page.request.get(`${baseURL}/api/v1/repositories`)
    const repoData = await repoResponse.json()
    
    console.log('✅ Temporary data directory confirmed - repositories list is empty')
    expect(repoData.repositories).toHaveLength(0)
  })
})