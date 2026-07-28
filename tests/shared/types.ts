/**
 * Response shapes the generated `@moatless/api-client` does not carry.
 *
 * Everything declared here is either a projection the suite builds for
 * readability (the trigger fixture), or an endpoint whose response type is not
 * exported by the generated client. Anything that exists upstream is imported,
 * not redeclared — the re-export block at the bottom of this file is how a test
 * gets those, so it imports every response shape from one place.
 */

export interface ListTasksFilters {
  limit?: number
  offset?: number
  status?: string
  repositoryId?: string
  tag?: string
}

export interface SearchByNameResult {
  files: Array<{ path: string; name: string }>
}

export interface TaskTriggerFixtureResponse {
  id: string
  connectionId: string
  adapterKind: string
  sourceId: string
  sourceName?: string
  repositoryId: string
  routingMode: string
  promptTemplate?: string
  taskName?: string
  active: boolean
  providerConfig?: Record<string, unknown>
  ongoingTaskId?: string
  createdBy: string
  createdAt: string
  updatedAt: string
}

/** What a test names when it wants a connection and a trigger over it. */
export interface TaskTriggerFixtureConfig {
  adapterKind?: string
  provider?: string
  sourceId: string
  sourceName?: string
  repositoryId: string
  routingMode: string
  promptTemplate?: string
  taskName?: string
  providerConfig?: Record<string, unknown>
  /** Connection kind, when the adapter has more than one. */
  connectionKind?: string
  /** What inbound events are matched on, when it is not the source id. */
  sourceMatcher?: string
}

export interface TriggerRuleResponse {
  repositoryId: string
  routingMode: string
  ongoingTaskId?: string | null
  prompt?: string | null
  taskName?: string | null
}

export interface AdapterEventTriggerSourceResponse {
  adapterKind: string
  connectionId: string
  sourceMatcher: string
  sourceName?: string | null
  adapterConfig?: Record<string, unknown> | null
}

export interface TriggerResponse {
  id: string
  name?: string | null
  kind: string
  active: boolean
  rule: TriggerRuleResponse
  source: AdapterEventTriggerSourceResponse | Record<string, never>
  createdBy: string
  createdAt: string
  updatedAt: string
}

// Re-exported so a test imports every response shape from one place.
export type {
  ActivationReach,
  ActivationResponse,
  AdapterConnectionResponse,
  AgentStatus,
  CreatePluginRequest,
  CreateRepositoryRequest,
  CreateVibeTaskRequest,
  EffectivePluginResponse,
  FileTreeNodeDto,
  FileTreeResponse,
  GetMessagesResponse,
  ListTasksResponse,
  PluginResponse,
  PluginSkillResponse,
  RepositoryResponse,
  SandboxStatus,
  SandboxStatusFullResponse,
  ServerEntry,
  ServerStatusResponse,
  SetActivationRequest,
  TaskResponse,
  TeamMemberResponse,
  TeamResponse,
  UiMessage,
  UpdateRepositoryRequest,
} from '@moatless/api-client/sandbox/model'
