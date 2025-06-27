import type { Event } from '@shared/types/event'

const API_BASE_URL = import.meta.env.VITE_API_URL || 'http://localhost:8000'

export interface NDJSONSSEClientOptions {
  taskId: string
  apiUrl?: string
  onEvent?: (event: Event) => void
  onError?: (error: Error) => void
  onConnect?: () => void
  onDisconnect?: () => void
  onTaskNotFound?: () => void
}

export class NDJSONSSEClient {
  private abortController: AbortController | null = null
  private taskId: string
  private apiUrl: string
  private onEvent?: (event: Event) => void
  private onError?: (error: Error) => void
  private onConnect?: () => void
  private onDisconnect?: () => void
  private onTaskNotFound?: () => void
  private reconnectTimeout: NodeJS.Timeout | null = null
  private lastEventId: string | null = null
  private isConnecting = false
  private reconnectAttempts = 0
  private maxReconnectAttempts = 10
  private reconnectDelay = 5000
  private shouldReconnect = true
  private taskNotFound = false
  private reader: ReadableStreamDefaultReader<Uint8Array> | null = null

  constructor(options: NDJSONSSEClientOptions) {
    this.taskId = options.taskId
    this.apiUrl = options.apiUrl || API_BASE_URL
    this.onEvent = options.onEvent
    this.onError = options.onError
    this.onConnect = options.onConnect
    this.onDisconnect = options.onDisconnect
    this.onTaskNotFound = options.onTaskNotFound
  }

  async connect(): Promise<void> {
    if (this.abortController || this.isConnecting || this.taskNotFound) {
      console.log(
        `[NDJSON-SSE] Connection already in progress or task not found for ${this.taskId}`,
      )
      return
    }

    console.log(`[NDJSON-SSE] Connecting to task ${this.taskId}`)
    this.isConnecting = true
    this.shouldReconnect = true

    try {
      const response = await this.makeRequest()
      await this.handleSuccessfulConnection(response)
    } catch (error) {
      this.handleConnectionError(error)
    }
  }

  private async makeRequest(): Promise<Response> {
    const url = this.buildUrl()
    console.log(`[NDJSON-SSE] Making request to ${url}`)
    this.abortController = new AbortController()

    const response = await fetch(url, {
      method: 'GET',
      credentials: 'include',
      signal: this.abortController.signal,
      headers: {
        Accept: 'application/x-ndjson',
      },
    })

    console.log(`[NDJSON-SSE] Response status: ${response.status}`)

    if (!response.ok) {
      if (response.status === 404) {
        this.handleTaskNotFound()
        throw new Error('Task not found')
      }
      throw new Error(`HTTP ${response.status}: ${response.statusText}`)
    }

    return response
  }

  private buildUrl(): string {
    let url = `${this.apiUrl}/api/v1/tasks/${this.taskId}/stream`
    if (this.lastEventId) {
      url += `?lastEventId=${encodeURIComponent(this.lastEventId)}`
    }
    return url
  }

  private async handleSuccessfulConnection(response: Response): Promise<void> {
    this.isConnecting = false
    this.reconnectAttempts = 0
    this.onConnect?.()

    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout)
      this.reconnectTimeout = null
    }

    if (response.body) {
      await this.readStream(response.body)
    }
  }

  private handleConnectionError(error: unknown): void {
    this.isConnecting = false

    if (error instanceof Error && error.name === 'AbortError') {
      if (this.shouldReconnect) {
        this.scheduleReconnect()
      }
    } else if (error instanceof Error) {
      this.onError?.(error)
      if (this.shouldReconnect) {
        this.scheduleReconnect()
      }
    }
  }

  private async readStream(body: ReadableStream<Uint8Array>): Promise<void> {
    const decoder = new TextDecoder()
    let buffer = ''

    try {
      this.reader = body.getReader()
      buffer = await this.processStreamData(decoder, buffer)
      this.processRemainingBuffer(buffer)
    } catch (error) {
      this.handleStreamError(error)
    } finally {
      this.cleanupStream()
    }
  }

  private async processStreamData(decoder: TextDecoder, buffer: string): Promise<string> {
    let currentBuffer = buffer

    while (true) {
      const { done, value } = await this.reader!.read()
      if (done) break

      currentBuffer += decoder.decode(value, { stream: true })
      currentBuffer = this.processBufferLines(currentBuffer)
    }

    return currentBuffer
  }

  private processBufferLines(buffer: string): string {
    const lines = buffer.split('\n')
    const remainingBuffer = lines.pop() || ''

    for (const line of lines) {
      if (line.trim()) {
        this.processLine(line)
      }
    }

    return remainingBuffer
  }

  private processRemainingBuffer(buffer: string): void {
    if (buffer.trim()) {
      this.processLine(buffer)
    }
  }

  private handleStreamError(error: unknown): void {
    if (error instanceof Error && error.name !== 'AbortError') {
      this.onError?.(error)
    }
  }

  private cleanupStream(): void {
    this.reader = null
    this.onDisconnect?.()

    if (this.shouldReconnect && !this.taskNotFound) {
      this.scheduleReconnect()
    }
  }

  private processLine(line: string): void {
    try {
      // Handle SSE format (if server sends it)
      if (line.startsWith('data: ')) {
        const data = line.slice(6)
        if (data === '[DONE]') {
          return
        }
        this.parseAndEmitEvent(data)
      } else if (line.startsWith('event: ')) {
        // Handle named events if needed
        return
      } else {
        // Handle plain NDJSON
        this.parseAndEmitEvent(line)
      }
    } catch (error) {
      console.error('Failed to process line:', error)
      this.onError?.(new Error('Failed to process stream data'))
    }
  }

  private parseAndEmitEvent(data: string): void {
    try {
      const parsed = JSON.parse(data)

      // Handle ping events (keep-alive)
      if (parsed.type === 'ping') {
        // Ignore ping events, they're just for keeping connection alive
        return
      }

      const event = parsed as Event

      // Store the last event ID for reconnection
      if (event.eventId) {
        this.lastEventId = event.eventId
      }

      this.onEvent?.(event)
    } catch (error) {
      console.error('Failed to parse event:', error)
      this.onError?.(new Error('Failed to parse event data'))
    }
  }

  disconnect(): void {
    this.shouldReconnect = false

    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout)
      this.reconnectTimeout = null
    }

    if (this.reader) {
      this.reader.cancel().catch(() => {
        // Ignore cancellation errors
      })
      this.reader = null
    }

    if (this.abortController) {
      this.abortController.abort()
      this.abortController = null
      this.onDisconnect?.()
    }
  }

  resetConnection(): void {
    this.taskNotFound = false
    this.shouldReconnect = true
    this.reconnectAttempts = 0
  }

  private handleTaskNotFound(): void {
    this.taskNotFound = true
    this.shouldReconnect = false
    this.onTaskNotFound?.()
    this.onError?.(new Error('Task not found (404)'))
    this.onDisconnect?.()
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimeout || !this.shouldReconnect) {
      return
    }

    this.reconnectAttempts++

    if (this.reconnectAttempts >= this.maxReconnectAttempts) {
      this.onError?.(new Error(`SSE connection failed after ${this.maxReconnectAttempts} attempts`))
      return
    }

    // Calculate exponential backoff with jitter
    const baseDelay = Math.min(this.reconnectDelay * 2 ** (this.reconnectAttempts - 1), 30000)
    const jitter = Math.random() * 1000
    const delay = Math.max(baseDelay + jitter, 1000)

    this.reconnectTimeout = setTimeout(() => {
      this.reconnectTimeout = null
      this.connect()
    }, delay)
  }

  isConnected(): boolean {
    return this.abortController !== null && !this.abortController.signal.aborted
  }
}
