export interface SSEConnection {
  id: string
  taskId: string
  controller: ReadableStreamDefaultController
  lastEventId?: string
  eventCount: number
}

export interface GlobalSSEConnection {
  id: string
  controller: ReadableStreamDefaultController
  eventCount: number
}

export class SSEConnectionManager {
  private connections: Map<string, Set<SSEConnection>> = new Map()
  private globalConnections: Map<string, GlobalSSEConnection> = new Map()
  private connectionCounter = 0

  connect(
    taskId: string,
    controller: ReadableStreamDefaultController,
    lastEventId?: string,
  ): string {
    const connectionId = `sse_${this.connectionCounter++}`

    const connection: SSEConnection = {
      id: connectionId,
      taskId,
      controller,
      lastEventId,
      eventCount: 0,
    }

    if (!this.connections.has(taskId)) {
      this.connections.set(taskId, new Set())
    }

    this.connections.get(taskId)!.add(connection)
    console.log(`SSE connection ${connectionId} established for task ${taskId}`)

    return connectionId
  }

  disconnect(taskId: string, connectionId: string): void {
    const taskConnections = this.connections.get(taskId)
    if (taskConnections) {
      const connection = Array.from(taskConnections).find((c) => c.id === connectionId)
      if (connection) {
        taskConnections.delete(connection)
        if (taskConnections.size === 0) {
          this.connections.delete(taskId)
        }
        console.log(`SSE connection ${connectionId} closed for task ${taskId}`)
      }
    }
  }

  getConnectionCount(taskId: string): number {
    return this.connections.get(taskId)?.size || 0
  }

  hasConnections(taskId: string): boolean {
    return this.getConnectionCount(taskId) > 0
  }

  getAllTaskIds(): string[] {
    return Array.from(this.connections.keys())
  }

  // Global connection methods
  connectGlobal(connectionId: string, controller: ReadableStreamDefaultController): void {
    const connection: GlobalSSEConnection = {
      id: connectionId,
      controller,
      eventCount: 0,
    }

    this.globalConnections.set(connectionId, connection)
    console.log(`Global SSE connection ${connectionId} established`)
  }

  disconnectGlobal(connectionId: string): void {
    const connection = this.globalConnections.get(connectionId)
    if (connection) {
      this.globalConnections.delete(connectionId)
      console.log(`Global SSE connection ${connectionId} closed`)
    }
  }

  getGlobalConnections(): GlobalSSEConnection[] {
    return Array.from(this.globalConnections.values())
  }

  getGlobalConnectionCount(): number {
    return this.globalConnections.size
  }

  getAllTaskConnectionCounts(): Record<string, number> {
    const counts: Record<string, number> = {}
    for (const [taskId, connections] of this.connections.entries()) {
      counts[taskId] = connections.size
    }
    return counts
  }

  incrementGlobalEventCount(): void {
    this.globalConnections.forEach((connection) => {
      connection.eventCount++
    })
  }

  // Get detailed connection info including event counts
  getTaskConnectionDetails(taskId: string): { connectionCount: number; totalEvents: number } {
    const connections = this.connections.get(taskId)
    if (!connections) {
      return { connectionCount: 0, totalEvents: 0 }
    }

    const totalEvents = Array.from(connections).reduce((sum, conn) => sum + conn.eventCount, 0)
    return {
      connectionCount: connections.size,
      totalEvents,
    }
  }

  getGlobalConnectionDetails(): { connectionCount: number; totalEvents: number } {
    const totalEvents = Array.from(this.globalConnections.values()).reduce(
      (sum, conn) => sum + conn.eventCount,
      0,
    )
    return {
      connectionCount: this.globalConnections.size,
      totalEvents,
    }
  }

  getAllConnectionStats(): {
    global: { connectionCount: number; totalEvents: number }
    tasks: Record<string, { connectionCount: number; totalEvents: number }>
    total: { connectionCount: number; totalEvents: number }
  } {
    const globalStats = this.getGlobalConnectionDetails()
    const taskStats: Record<string, { connectionCount: number; totalEvents: number }> = {}

    let totalConnections = globalStats.connectionCount
    let totalEvents = globalStats.totalEvents

    for (const taskId of this.connections.keys()) {
      const details = this.getTaskConnectionDetails(taskId)
      taskStats[taskId] = details
      totalConnections += details.connectionCount
      totalEvents += details.totalEvents
    }

    return {
      global: globalStats,
      tasks: taskStats,
      total: {
        connectionCount: totalConnections,
        totalEvents,
      },
    }
  }

  // Graceful shutdown - close all connections
  async gracefulShutdown(): Promise<void> {
    const safeLog = (message: string) => {
      try {
        console.log(message)
      } catch {
        // Ignore console errors during shutdown
      }
    }

    safeLog('Starting SSE connection manager graceful shutdown...')

    // Close all task-specific connections
    for (const [taskId, connections] of this.connections.entries()) {
      safeLog(`Closing ${connections.size} connections for task ${taskId}`)
      for (const connection of connections) {
        try {
          // Send a close event to notify clients
          const encoder = new TextEncoder()
          connection.controller.enqueue(
            encoder.encode(`event: close\ndata: {"reason": "server_shutdown"}\n\n`),
          )
          // Close the connection
          connection.controller.close()
        } catch (_error) {
          // Silently ignore errors during shutdown
        }
      }
      connections.clear()
    }
    this.connections.clear()

    // Close all global connections
    safeLog(`Closing ${this.globalConnections.size} global connections`)
    for (const [, connection] of this.globalConnections.entries()) {
      try {
        // Send a close event to notify clients
        const encoder = new TextEncoder()
        connection.controller.enqueue(
          encoder.encode(`event: close\ndata: {"reason": "server_shutdown"}\n\n`),
        )
        // Close the connection
        connection.controller.close()
      } catch (_error) {
        // Silently ignore errors during shutdown
      }
    }
    this.globalConnections.clear()

    safeLog('SSE connection manager graceful shutdown completed')
  }
}

export const sseManager = new SSEConnectionManager()
