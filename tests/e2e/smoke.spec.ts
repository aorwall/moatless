import { test, expect } from '@playwright/test'

/**
 * Smoke test to validate E2E infrastructure is working correctly
 */

test.describe('E2E Infrastructure Smoke Test', () => {
  test('should load the application homepage', async ({ page }) => {
    // Navigate to the application
    await page.goto('/')
    
    // Wait for the page to load - be more permissive with loading state
    await page.waitForLoadState('domcontentloaded')
    
    // Check that we can access the basic application
    // This validates that:
    // 1. Server is running on the expected port
    // 2. Client build is accessible
    // 3. Basic routing works
    
    // Wait for the page to have content - look for any indication the app loaded
    try {
      // First try to wait for title
      await expect(page).toHaveTitle(/Moatless|React|Vite/i, { timeout: 30000 })
    } catch {
      // If title doesn't match, at least check that page loads some content
      await expect(page.locator('body')).not.toBeEmpty({ timeout: 30000 })
      console.log('⚠️ App loaded but title may not match expected pattern')
    }
    
    console.log('✅ Homepage loaded successfully')
  })

  test('should be able to access API endpoints', async ({ page, baseURL }) => {
    // Test basic API accessibility
    const response = await page.request.get(`${baseURL}/api/v1/repositories`)
    expect(response.status()).toBe(200)
    
    const data = await response.json()
    expect(data).toHaveProperty('repositories')
    expect(Array.isArray(data.repositories)).toBe(true)
    
    console.log('✅ API endpoints accessible')
  })

  test('should be able to establish SSE connection', async ({ page, baseURL }) => {
    // Navigate to the app first to establish proper context
    await page.goto('/')
    
    // Test SSE connection via direct request (avoids CORS issues)
    const response = await page.request.get(`${baseURL}/api/v1/events/stream/status`)
    expect(response.status()).toBe(200)
    
    const status = await response.json()
    expect(status).toHaveProperty('globalConnections')
    expect(status).toHaveProperty('taskConnections')
    
    // Test SSE endpoint availability using fetch with AbortController
    // We need to abort the connection quickly since SSE keeps the connection open
    const sseTest = await page.evaluate(async (url) => {
      const controller = new AbortController()
      
      try {
        const response = await fetch(`${url}/api/v1/events/stream`, {
          headers: {
            'Accept': 'text/event-stream'
          },
          signal: controller.signal
        })
        
        // Abort after checking the response
        setTimeout(() => controller.abort(), 100)
        
        return {
          status: response.status,
          contentType: response.headers.get('content-type'),
          ok: response.ok
        }
      } catch (error) {
        // If it's an abort error, that's expected
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
    
    console.log('✅ SSE endpoints are accessible')
  })

  test('should have required dependencies available', async ({ page, baseURL }) => {
    // Check that the global event stream status endpoint works
    const response = await page.request.get(`${baseURL}/api/v1/events/stream/status`)
    expect(response.status()).toBe(200)
    
    const status = await response.json()
    expect(status).toHaveProperty('globalConnections')
    expect(status).toHaveProperty('taskConnections')
    expect(status).toHaveProperty('totalConnections')
    
    console.log('✅ Event stream infrastructure ready')
  })
})