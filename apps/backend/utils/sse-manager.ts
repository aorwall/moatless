import type { FastifyReply, FastifyRequest } from 'fastify'
import type { EventType } from '@moatless/shared/types/event'

export interface SSEConnection {
  id: string
  taskId: string
  reply: FastifyReply
  request: FastifyRequest
  lastEventId?: string
}

export interface GlobalSSEConnection {
  id: string
  reply: FastifyReply
  request: FastifyRequest
  filters?: {
    types?: EventType[]
  }
}

export class SSEConnectionManager {
  private connections: Map<string, Set<SSEConnection>> = new Map()
  private globalConnections: Map<string, GlobalSSEConnection> = new Map()
  private connectionCounter = 0

  connect(
    taskId: string,
    request: FastifyRequest,
    reply: FastifyReply,
    lastEventId?: string,
  ): string {
    const connectionId = `sse_${this.connectionCounter++}`

    const connection: SSEConnection = {
      id: connectionId,
      taskId,
      reply,
      request,
      lastEventId,
    }

    if (!this.connections.has(taskId)) {
      this.connections.set(taskId, new Set())
    }

    this.connections.get(taskId)!.add(connection)
    request.log.info(`SSE connection ${connectionId} established for task ${taskId}`)

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
        connection.request.log.info(`SSE connection ${connectionId} closed for task ${taskId}`)
      }
    }
  }

  getConnections(taskId: string): SSEConnection[] {
    return Array.from(this.connections.get(taskId) || [])
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
  connectGlobal(
    connectionId: string,
    request: FastifyRequest,
    reply: FastifyReply,
    filters?: GlobalSSEConnection['filters'],
  ): void {
    const connection: GlobalSSEConnection = {
      id: connectionId,
      reply,
      request,
      filters,
    }

    this.globalConnections.set(connectionId, connection)
    request.log.info(`Global SSE connection ${connectionId} established`)
  }

  disconnectGlobal(connectionId: string): void {
    const connection = this.globalConnections.get(connectionId)
    if (connection) {
      this.globalConnections.delete(connectionId)
      connection.request.log.info(`Global SSE connection ${connectionId} closed`)
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
}

export const sseManager = new SSEConnectionManager()
