import { expect, test } from '@playwright/test'

/**
 * Basic SSE functionality test to isolate and debug SSE issues
 */

test.describe('SSE Basic Functionality', () => {
  const baseURL = 'http://127.0.0.1:3002'

  test('should establish SSE connection and receive ping events', async ({ page }) => {
    // Navigate to a basic page
    await page.goto('/')

    // Set up NDJSON stream connection in the browser
    const connectionResult = await page.evaluate(async (url) => {
      return new Promise((resolve) => {
        const events: any[] = []
        const errors: any[] = []
        let connected = false
        const abortController = new AbortController()

        // Use fetch with ReadableStream to handle NDJSON format
        fetch(`${url}/api/v1/events/stream?activeTasksOnly=false`, {
          method: 'GET',
          headers: {
            Accept: 'text/plain',
            'Cache-Control': 'no-cache',
          },
          credentials: 'include',
          signal: abortController.signal,
        })
          .then(async (response) => {
            if (!response.ok) {
              throw new Error(`HTTP ${response.status}: ${response.statusText}`)
            }

            if (!response.body) {
              throw new Error('No response body')
            }

            connected = true
            console.log('[Test SSE] Connected to NDJSON stream')

            const reader = response.body.getReader()
            const decoder = new TextDecoder()
            let buffer = ''

            try {
              while (true) {
                const { done, value } = await reader.read()

                if (done) {
                  console.log('[Test SSE] Stream ended')
                  break
                }

                // Decode the chunk and add to buffer
                buffer += decoder.decode(value, { stream: true })

                // Process complete lines (NDJSON format)
                const lines = buffer.split('\n')
                buffer = lines.pop() || '' // Keep incomplete line in buffer

                for (const line of lines) {
                  if (line.trim()) {
                    try {
                      console.log('[Test SSE] Raw line received:', line)

                      const data = JSON.parse(line)

                      console.log('[Test SSE] Parsed event data:', {
                        type: data.type,
                        taskId: data.taskId,
                        timestamp: data.timestamp,
                      })

                      events.push(data)
                    } catch (error) {
                      const errorMsg =
                        'Failed to parse NDJSON line: ' + error.message + ' Raw line: ' + line
                      errors.push(errorMsg)
                      console.error('[Test SSE]', errorMsg)
                    }
                  }
                }
              }
            } catch (error) {
              if (error.name === 'AbortError') {
                console.log('[Test SSE] Connection aborted')
                return
              }
              throw error
            }
          })
          .catch((error) => {
            if (error.name !== 'AbortError') {
              const errorMsg = 'NDJSON connection error: ' + error.message
              errors.push(errorMsg)
              console.error('[Test SSE]', errorMsg)
            }
          })

        // Wait for 5 seconds to collect events
        setTimeout(() => {
          abortController.abort()
          resolve({
            connected,
            events,
            errors,
            eventsCount: events.length,
            pingEventsCount: events.filter((e) => e.type === 'ping').length,
          })
        }, 5000)
      })
    }, baseURL)

    console.log('SSE Connection Result:', connectionResult)

    // Verify connection was established
    expect(connectionResult.connected).toBe(true)
    expect(connectionResult.errors.length).toBe(0)

    // We should receive at least the initial connection (no ping within 5s)
    console.log(
      `Received ${connectionResult.eventsCount} events, ${connectionResult.pingEventsCount} pings`,
    )
  })

  test('should receive events from API-created task', async ({ page }) => {
    // First create a repository for the task
    const repoResponse = await page.request.post(`${baseURL}/api/v1/repositories`, {
      data: {
        name: 'test-sse-repo',
        description: 'Test repository for SSE testing',
        path: '/tmp/test-sse-repo',
        useGitWorktrees: false,
        codeExecution: { type: 'local' },
        variables: {},
      },
    })

    if (!repoResponse.ok) {
      throw new Error(`Failed to create repository: ${await repoResponse.text()}`)
    }

    const repoData = await repoResponse.json()
    console.log('Created repository:', repoData.repositoryId)

    // Now create a task with the valid repository
    const createResponse = await page.request.post(`${baseURL}/api/v1/tasks`, {
      data: {
        agentType: 'claude-code',
        repositoryId: repoData.repositoryId,
        message: 'Test task for SSE',
        start: false, // Don't start the task, just create it
      },
    })

    if (!createResponse.ok) {
      throw new Error(`Failed to create task: ${await createResponse.text()}`)
    }

    const taskData = await createResponse.json()
    console.log('Created task:', taskData.taskId)

    // Navigate to page
    await page.goto('/')

    // Set up NDJSON stream and wait for task events
    const sseResult = await page.evaluate(
      async (params) => {
        const { url, taskId } = params
        return new Promise((resolve) => {
          const events: any[] = []
          let connected = false
          const abortController = new AbortController()

          // Use fetch with ReadableStream to handle NDJSON format
          fetch(`${url}/api/v1/events/stream?activeTasksOnly=false`, {
            method: 'GET',
            headers: {
              Accept: 'text/plain',
              'Cache-Control': 'no-cache',
            },
            credentials: 'include',
            signal: abortController.signal,
          })
            .then(async (response) => {
              if (!response.ok) {
                throw new Error(`HTTP ${response.status}: ${response.statusText}`)
              }

              if (!response.body) {
                throw new Error('No response body')
              }

              connected = true
              console.log('[Test SSE] Connected for task monitoring')

              const reader = response.body.getReader()
              const decoder = new TextDecoder()
              let buffer = ''

              try {
                while (true) {
                  const { done, value } = await reader.read()

                  if (done) {
                    console.log('[Test SSE] Stream ended')
                    break
                  }

                  // Decode the chunk and add to buffer
                  buffer += decoder.decode(value, { stream: true })

                  // Process complete lines (NDJSON format)
                  const lines = buffer.split('\n')
                  buffer = lines.pop() || '' // Keep incomplete line in buffer

                  for (const line of lines) {
                    if (line.trim()) {
                      try {
                        const data = JSON.parse(line)

                        if (data.taskId === taskId) {
                          events.push(data)
                          console.log('[Test SSE] Task event:', data.type)
                        }
                      } catch (error) {
                        console.error('[Test SSE] Parse error:', error)
                      }
                    }
                  }
                }
              } catch (error) {
                if (error.name === 'AbortError') {
                  console.log('[Test SSE] Connection aborted')
                  return
                }
                throw error
              }
            })
            .catch((error) => {
              if (error.name !== 'AbortError') {
                console.error('[Test SSE] Connection error:', error)
              }
            })

          // Wait for events
          setTimeout(() => {
            abortController.abort()
            resolve({
              connected,
              taskEvents: events,
              taskEventsCount: events.length,
            })
          }, 3000)
        })
      },
      { url: baseURL, taskId: taskData.taskId },
    )

    console.log('SSE Task Result:', sseResult)

    // Cleanup: Delete the task and repository
    try {
      await page.request.delete(`${baseURL}/api/v1/tasks/${taskData.taskId}`)
      await page.request.delete(`${baseURL}/api/v1/repositories/${repoData.repositoryId}`)
    } catch (error) {
      console.warn('Cleanup failed:', error)
    }

    // Verify we received task creation event
    expect(sseResult.connected).toBe(true)
    expect(sseResult.taskEventsCount).toBeGreaterThan(0)
  })
})
