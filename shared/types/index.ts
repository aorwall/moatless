export interface Task {
  taskId: string
  createdAt: Date
  lastMessageAt: Date
  agentType: string
  environmentId: string
  status: 'active' | 'completed' | 'error'
  numTurns: number
  totalCostUsd: number
}

export interface Environment {
  id: string
  name: string
  type: string
  repository?: {
    type: 'local' | 'remote'
    path?: string
    url?: string
  }
}

export interface Event {
  eventId: string
  type: string
  taskId: string
  timestamp: Date
  isError: boolean
  payload: any
}