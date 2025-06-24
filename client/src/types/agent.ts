export interface AgentOptions {
  [key: string]: unknown
}

export interface AgentTriggerRequest {
  agentType: string
  message: string
  environmentId: string
  branch?: string
}

export interface AgentTriggerResponse {
  taskId: string
  status: string
  stream_url?: string
}
