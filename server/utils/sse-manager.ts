import type { FastifyReply, FastifyRequest } from 'fastify'

export interface SSEConnection {
  id: string
  taskId: string
  reply: FastifyReply
  request: FastifyRequest
  lastEventId?: string
}

export class SSEConnectionManager {
  private connections: Map<string, Set<SSEConnection>> = new Map()
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
}

export const sseManager = new SSEConnectionManager()
