import type { Page } from '@playwright/test'

export interface Event {
  eventId: string
  taskId: string
  type: string
  timestamp: string
  iterationId?: string
  isError?: boolean
  payload?: any
}

export interface EventMonitor {
  events: Event[]
  errors: string[]
  isConnected: boolean
  cleanup: () => Promise<void>
}

/**
 * Monitor events from the backend API directly
 */
export class APIEventMonitor implements EventMonitor {
  public events: Event[] = []
  public errors: string[] = []
  public isConnected = false

  private intervalId: NodeJS.Timeout | null = null
  private seenEventIds = new Set<string>()
  private baseURL: string
  private taskId?: string

  constructor(baseURL: string, taskId?: string) {
    this.baseURL = baseURL
    this.taskId = taskId
  }

  async start(): Promise<void> {
    this.isConnected = true

    // Poll for events every 500ms
    this.intervalId = setInterval(async () => {
      try {
        await this.fetchEvents()
      } catch (error) {
        this.errors.push(`API polling error: ${error}`)
      }
    }, 500)
  }

  private async fetchEvents(): Promise<void> {
    const url = this.taskId
      ? `${this.baseURL}/api/v1/tasks/${this.taskId}/events`
      : `${this.baseURL}/api/v1/events`

    const response = await fetch(url)
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${response.statusText}`)
    }

    const { events } = await response.json()

    // Process only new events
    for (const event of events) {
      if (!this.seenEventIds.has(event.eventId)) {
        this.seenEventIds.add(event.eventId)
        this.events.push(event)
      }
    }
  }

  async cleanup(): Promise<void> {
    this.isConnected = false
    if (this.intervalId) {
      clearInterval(this.intervalId)
      this.intervalId = null
    }
  }
}

/**
 * Monitor events from the frontend SSE connection using browser APIs
 */
export class SSEEventMonitor implements EventMonitor {
  public events: Event[] = []
  public errors: string[] = []
  public isConnected = false

  private page: Page
  private baseURL: string
  private taskId?: string
  private connectionScript?: string

  constructor(page: Page, baseURL: string, taskId?: string) {
    this.page = page
    this.baseURL = baseURL
    this.taskId = taskId
  }

  async start(): Promise<void> {
    // Inject SSE monitoring script into the page
    await this.page.addInitScript(() => {
      // Store events globally so we can access them from Playwright
      ;(window as any).sseEvents =
        [](window as any).sseErrors =
        [](window as any).sseConnected =
          false
    })

    // Start the NDJSON stream connection in the browser context (matching frontend implementation)
    this.connectionScript = `
      const events = [];
      const errors = [];
      let connected = false;
      let abortController = new AbortController();

      const url = '${this.baseURL}/api/v1/events/stream?activeTasksOnly=true';
      
      // Use fetch with ReadableStream to handle NDJSON format (matching frontend)
      fetch(url, {
        method: 'GET',
        headers: {
          'Accept': 'text/plain',
          'Cache-Control': 'no-cache',
        },
        credentials: 'include',
        signal: abortController.signal,
      })
      .then(async (response) => {
        if (!response.ok) {
          throw new Error(\`HTTP \${response.status}: \${response.statusText}\`);
        }

        if (!response.body) {
          throw new Error('No response body');
        }

        connected = true;
        window.sseConnected = true;
        console.log('[E2E SSE] Connected to NDJSON stream');

        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        let buffer = '';

        try {
          while (true) {
            const { done, value } = await reader.read();
            
            if (done) {
              console.log('[E2E SSE] Stream ended');
              break;
            }

            // Decode the chunk and add to buffer
            buffer += decoder.decode(value, { stream: true });

            // Process complete lines (NDJSON format)
            const lines = buffer.split('\\n');
            buffer = lines.pop() || ''; // Keep incomplete line in buffer

            for (const line of lines) {
              if (line.trim()) {
                try {
                  console.log('[E2E SSE] Raw line received:', line);
                  
                  const data = JSON.parse(line);
                  
                  console.log('[E2E SSE] Parsed event data:', {
                    type: data.type,
                    taskId: data.taskId,
                    eventId: data.eventId || data.id,
                    timestamp: data.timestamp,
                  });

                  // Ignore ping events
                  if (data.type === 'ping') {
                    console.log('[E2E SSE] Ignoring ping event');
                    continue;
                  }

                  events.push(data);
                  window.sseEvents = [...events];
                  console.log('[E2E SSE] Event received:', data.type, data.eventId || data.id);
                } catch (error) {
                  const errorMsg = 'Failed to parse NDJSON line: ' + error.message + ' Raw line: ' + line;
                  errors.push(errorMsg);
                  window.sseErrors = [...errors];
                  console.error('[E2E SSE]', errorMsg);
                }
              }
            }
          }
        } catch (error) {
          if (error.name === 'AbortError') {
            console.log('[E2E SSE] Connection aborted');
            return;
          }
          throw error;
        }
      })
      .catch((error) => {
        connected = false;
        window.sseConnected = false;
        const errorMsg = 'NDJSON connection error: ' + error.message;
        errors.push(errorMsg);
        window.sseErrors = [...errors];
        console.error('[E2E SSE]', errorMsg);
      });

      // Store cleanup function globally
      window.cleanupSSE = () => {
        abortController.abort();
        connected = false;
        window.sseConnected = false;
        console.log('[E2E SSE] Connection cleaned up');
      };
    `

    await this.page.evaluate(this.connectionScript)

    // Wait a moment for connection to establish
    await new Promise((resolve) => setTimeout(resolve, 1000))

    this.isConnected = await this.page.evaluate(() => (window as any).sseConnected)
  }

  async getLatestEvents(): Promise<Event[]> {
    const events = await this.page.evaluate(() => (window as any).sseEvents || [])
    this.events = events

    // Also check for any errors
    const errors = await this.page.evaluate(() => (window as any).sseErrors || [])
    if (errors.length > 0) {
      console.log('[SSE Monitor] Errors detected:', errors)
    }

    return events
  }

  async getLatestErrors(): Promise<string[]> {
    const errors = await this.page.evaluate(() => (window as any).sseErrors || [])
    this.errors = errors
    return errors
  }

  async checkConnection(): Promise<boolean> {
    this.isConnected = await this.page.evaluate(() => (window as any).sseConnected || false)
    return this.isConnected
  }

  async cleanup(): Promise<void> {
    try {
      await this.page.evaluate(() => {
        if ((window as any).cleanupSSE) {
          ;(window as any).cleanupSSE()
        }
      })
    } catch (error) {
      this.errors.push(`Cleanup error: ${error}`)
    }
    this.isConnected = false
  }
}

/**
 * Monitor global event stream status
 */
export async function getEventStreamStatus(baseURL: string): Promise<{
  globalConnections: number
  taskConnections: Record<string, number>
  totalConnections: number
}> {
  const response = await fetch(`${baseURL}/api/v1/events/stream/status`)
  if (!response.ok) {
    throw new Error(`Failed to get stream status: ${response.status}`)
  }
  return await response.json()
}

/**
 * Utility function to wait for specific events
 */
export async function waitForEvents(
  monitor: EventMonitor,
  predicate: (events: Event[]) => boolean,
  timeoutMs = 30000,
  checkIntervalMs = 500,
): Promise<Event[]> {
  const startTime = Date.now()

  while (Date.now() - startTime < timeoutMs) {
    // Update events if it's an SSE monitor
    if (monitor instanceof SSEEventMonitor) {
      await monitor.getLatestEvents()
    }

    if (predicate(monitor.events)) {
      return monitor.events
    }

    await new Promise((resolve) => setTimeout(resolve, checkIntervalMs))
  }

  throw new Error(`Timeout waiting for events. Current events: ${monitor.events.length}`)
}

/**
 * Utility function to wait for a specific event type
 */
export async function waitForEventType(
  monitor: EventMonitor,
  eventType: string,
  timeoutMs = 30000,
): Promise<Event> {
  await waitForEvents(monitor, (events) => events.some((e) => e.type === eventType), timeoutMs)

  const event = monitor.events.find((e) => e.type === eventType)
  if (!event) {
    throw new Error(`Event of type ${eventType} not found`)
  }
  return event
}

/**
 * Utility function to wait for task completion
 */
export async function waitForTaskResult(
  monitor: EventMonitor,
  taskId: string,
  timeoutMs = 60000,
): Promise<{ success: boolean; event: Event }> {
  await waitForEvents(
    monitor,
    (events) => events.some((e) => e.taskId === taskId && e.type === 'agent' && e.subType === 'completed'),
    timeoutMs,
  )

  const resultEvent = monitor.events.find((e) => e.taskId === taskId && e.type === 'agent' && e.subType === 'completed')
  if (!resultEvent) {
    throw new Error(`Result event for task ${taskId} not found`)
  }

  return {
    success: !resultEvent.isError,
    event: resultEvent,
  }
}
