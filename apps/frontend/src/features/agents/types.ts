export interface AgentModel {
  id: string
  label: string
}

export interface AgentDescriptor {
  id: string
  label: string
  installed: boolean
  defaultModel: string
  models: AgentModel[]
  missingCredentialHint?: string
}

export interface ListAgentsResponse {
  agents: AgentDescriptor[]
}
