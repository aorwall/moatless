import { test, expect } from '@playwright/test'

/**
 * Basic SSE functionality test to isolate and debug SSE issues
 */

test.describe('SSE Basic Functionality', () => {
  const baseURL = 'http://127.0.0.1:3002'

  test('should establish SSE connection and receive ping events', async ({ page }) => {
    // Navigate to a basic page
    await page.goto('/')
    
    // Set up SSE connection in the browser
    const connectionResult = await page.evaluate(async (url) => {
      return new Promise((resolve) => {
        const events: any[] = []
        const errors: any[] = []
        let connected = false
        
        const eventSource = new EventSource(`${url}/api/v1/events/stream?activeTasksOnly=false`)
        
        eventSource.onopen = () => {
          connected = true
          console.log('[Test SSE] Connected')
        }
        
        eventSource.onmessage = (event) => {
          console.log('[Test SSE] Message received:', event.data)
          try {
            const data = JSON.parse(event.data)
            events.push(data)
          } catch (error) {
            errors.push(`Parse error: ${error}`)
          }
        }
        
        eventSource.onerror = (error) => {
          console.error('[Test SSE] Error:', error)
          errors.push(`Connection error: ${error.type}`)
        }
        
        // Wait for 5 seconds to collect events
        setTimeout(() => {
          eventSource.close()
          resolve({
            connected,
            events,
            errors,
            eventsCount: events.length,
            pingEventsCount: events.filter(e => e.type === 'ping').length
          })
        }, 5000)
      })
    }, baseURL)
    
    console.log('SSE Connection Result:', connectionResult)
    
    // Verify connection was established
    expect(connectionResult.connected).toBe(true)
    expect(connectionResult.errors.length).toBe(0)
    
    // We should receive at least the initial connection (no ping within 5s)
    console.log(`Received ${connectionResult.eventsCount} events, ${connectionResult.pingEventsCount} pings`)
  })

  test('should receive events from API-created task', async ({ page }) => {
    // First create a task via API
    const createResponse = await page.request.post(`${baseURL}/api/v1/tasks`, {
      data: {
        agentType: 'claude-code',
        repositoryId: 'test-repo-id', // This will fail but generate an error event
        message: 'Test task for SSE',
        start: false
      }
    })
    
    const taskData = await createResponse.json()
    console.log('Created task:', taskData.taskId)
    
    // Navigate to page
    await page.goto('/')
    
    // Set up SSE and wait for task events
    const sseResult = await page.evaluate(async (params) => {
      const { url, taskId } = params
      return new Promise((resolve) => {
        const events: any[] = []
        let connected = false
        
        const eventSource = new EventSource(`${url}/api/v1/events/stream?activeTasksOnly=false`)
        
        eventSource.onopen = () => {
          connected = true
          console.log('[Test SSE] Connected for task monitoring')
        }
        
        eventSource.onmessage = (event) => {
          try {
            const data = JSON.parse(event.data)
            if (data.taskId === taskId) {
              events.push(data)
              console.log('[Test SSE] Task event:', data.type)
            }
          } catch (error) {
            console.error('[Test SSE] Parse error:', error)
          }
        }
        
        // Wait for events
        setTimeout(() => {
          eventSource.close()
          resolve({
            connected,
            taskEvents: events,
            taskEventsCount: events.length
          })
        }, 3000)
      })
    }, { url: baseURL, taskId: taskData.taskId })
    
    console.log('SSE Task Result:', sseResult)
    
    // Verify we received task creation event
    expect(sseResult.connected).toBe(true)
    expect(sseResult.taskEventsCount).toBeGreaterThan(0)
  })
})